//! `--live` end-to-end (docs/HEW_API.md §11.2, §12): drives the real
//! `HEW_RUNTIME_DIR`-reading discovery path (`hew_cli::live::discover`,
//! not the fixture-directory-only unit tests in `crates/hew-cli/src/live.rs`)
//! together with a real `UnixListener` faking a desktop instance, and the
//! full `hew_cli::run::dispatch_live` client path on top of it — proving
//! discovery, stale-file cleanup, multi-candidate disambiguation, and the
//! handshake-then-forward transport all compose correctly through the
//! public entry points a real invocation would use, spawn-free (no
//! subprocess, no piped stdio).
//!
//! `HEW_RUNTIME_DIR` mutates process-global environment state, so
//! everything here runs as one `#[test]` function to avoid any race with
//! a sibling test in this same binary.
//!
//! Unix-only: built entirely on `std::os::unix::net::UnixListener` to fake
//! a desktop instance. The Windows named-pipe transport's own end-to-end
//! coverage lives as `#[cfg(windows)]` tests inside `src/live.rs` instead
//! (it cannot reuse this file's fake-instance harness, since that harness
//! is itself unix-socket-specific).
#![cfg(unix)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

/// `HEW_RUNTIME_DIR` is process-wide, and more than one test in this
/// binary points it at its own fixture directory. Cargo runs them as
/// threads of ONE process, so they must not overlap: each takes this
/// lock for its whole scenario.
static RUNTIME_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A short bind path — `AF_UNIX` paths are capped at `SUN_LEN` (~104 bytes
/// on macOS/BSD), so this binds directly under `/tmp` rather than under
/// `std::env::temp_dir()`'s (much longer, on macOS) per-process directory.
fn short_socket_path(tag: &str) -> PathBuf {
    PathBuf::from("/tmp").join(format!(
        "hew-live-cli-test-{tag}-{}.sock",
        std::process::id()
    ))
}

/// One accept -> hello -> attach-gated command loop, matching what a real
/// desktop instance's connection handler does (docs/HEW_API.md §11.2,
/// §4.2, §10): after `hello`, every command but `hew.doc.attach` is
/// refused `-32002 no document attached` until the client sends it —
/// exactly the gate `crates/api/src/dispatch.rs`'s `attached` flag
/// enforces for real, and specifically the piece the OLD version of this
/// fake didn't model (it answered every method with a canned result
/// unconditionally, which is what let the "dispatch --live never
/// attaches" and "mcp --live never attaches" defects through review — the
/// test tautologically restated the client instead of modeling the
/// server's own state machine). Runs until the client disconnects (EOF —
/// a live session never sends an explicit close frame, it just drops the
/// socket) and returns every method name it saw after hello, in order, so
/// a test can assert the exact sequence a caller sent.
fn spawn_attach_gated_fake_instance(
    socket_path: PathBuf,
    expected_token: &'static str,
) -> std::thread::JoinHandle<Vec<String>> {
    let listener = UnixListener::bind(&socket_path).expect("bind the fake instance socket");
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept the one connection");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        let mut seen = Vec::new();

        let mut line = String::new();
        reader.read_line(&mut line).expect("read the hello frame");
        let hello: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(hello["method"], "hew.meta.hello");
        assert_eq!(
            hello["params"]["token"], expected_token,
            "the discovery file's token must reach the wire"
        );
        let hello_reply = json!({
            "jsonrpc": "2.0", "id": hello["id"],
            "result": {
                "protocol": 1, "app": { "name": "hew", "version": "0.5.0" },
                "profile": "app", "encoding": "json", "documents": [],
            },
        });
        writeln!(writer, "{hello_reply}").unwrap();

        let mut attached = false;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).expect("read a frame");
            if n == 0 {
                break; // the client disconnected — the session is over
            }
            let frame: Value = serde_json::from_str(line.trim_end()).unwrap();
            let method = frame["method"].as_str().unwrap_or("").to_string();
            seen.push(method.clone());
            let reply = if method == "hew.doc.attach" {
                attached = true;
                json!({ "jsonrpc": "2.0", "id": frame["id"], "result": {} })
            } else if !attached {
                json!({
                    "jsonrpc": "2.0", "id": frame["id"],
                    "error": { "code": -32002, "message": "no document attached" },
                })
            } else if method == "hew.doc.save"
                || method == "hew.doc.export"
                || method == "hew.doc.transact"
            {
                // What a live host really answers: it has no filesystem,
                // so it refuses a `path` and otherwise hands the bytes
                // back for the client to write. If the client forwards a
                // path instead of stripping it, this refuses — which is
                // exactly the bug the test below guards.
                let forwarded_path = frame["params"].get("path").is_some()
                    || frame["params"]["commands"][0]["params"]
                        .get("path")
                        .is_some();
                if forwarded_path {
                    json!({
                        "jsonrpc": "2.0", "id": frame["id"],
                        "error": {
                            "code": -32000, "message": "refused",
                            "data": { "refusal": "host_capability_missing" },
                        },
                    })
                } else if method == "hew.doc.transact" {
                    // A transact envelope's result wraps each command's
                    // own result — the shape §6.4's one-command form
                    // produces, and the only shape MCP can send.
                    json!({
                        "jsonrpc": "2.0", "id": frame["id"],
                        "result": { "label": "t", "results": [{ "bytes_base64": "aGV3IQ==" }] },
                    })
                } else {
                    // A BARE save/export answers its own result directly,
                    // NOT wrapped in `results` — matching what
                    // crates/api actually returns, so this fake cannot
                    // certify a client that only understands the wrapped
                    // shape. "hew!" in base64.
                    json!({
                        "jsonrpc": "2.0", "id": frame["id"],
                        "result": { "bytes_base64": "aGV3IQ==" },
                    })
                }
            } else {
                json!({ "jsonrpc": "2.0", "id": frame["id"], "result": { "tree": [] } })
            };
            writeln!(writer, "{reply}").unwrap();
        }
        seen
    })
}

/// A fake instance that hello's normally but refuses `hew.doc.attach`
/// itself (an application-side policy refusal — e.g. no document open at
/// all) — proves the attach step fails the whole connection cleanly
/// rather than falling through to dispatch the caller's real command
/// anyway.
fn spawn_attach_refusing_fake_instance(
    socket_path: PathBuf,
    expected_token: &'static str,
) -> std::thread::JoinHandle<()> {
    let listener = UnixListener::bind(&socket_path).expect("bind the fake instance socket");
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept the one connection");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;

        let mut line = String::new();
        reader.read_line(&mut line).expect("read the hello frame");
        let hello: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(hello["method"], "hew.meta.hello");
        assert_eq!(hello["params"]["token"], expected_token);
        let hello_reply = json!({
            "jsonrpc": "2.0", "id": hello["id"],
            "result": {
                "protocol": 1, "app": { "name": "hew", "version": "0.5.0" },
                "profile": "app", "encoding": "json", "documents": [],
            },
        });
        writeln!(writer, "{hello_reply}").unwrap();

        let mut line2 = String::new();
        reader.read_line(&mut line2).expect("read the attach frame");
        let attach: Value = serde_json::from_str(line2.trim_end()).unwrap();
        assert_eq!(attach["method"], "hew.doc.attach");
        let refusal = json!({
            "jsonrpc": "2.0", "id": attach["id"],
            "error": { "code": -32000, "message": "no document is open" },
        });
        writeln!(writer, "{refusal}").unwrap();
        // The client must not send anything else after an attach refusal.
    })
}

fn write_instance_file(dir: &std::path::Path, pid: u32, socket: &str, token: &str) {
    // Discovery trusts an entry only when the socket it advertises is
    // ours, so a fixture must materialize the path (a plain file suffices
    // for entries nothing ever dials).
    if !std::path::Path::new(socket).exists() {
        let _ = std::fs::write(socket, b"");
    }
    let body = json!({ "socket": socket, "token": token, "pid": pid, "version": "0.5.0" });
    std::fs::write(
        dir.join(format!("instance-{pid}.json")),
        serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();
}

#[test]
fn discovery_and_dispatch_live_work_through_hew_runtime_dir_end_to_end() {
    let fixture_dir = std::env::temp_dir().join(format!(
        "hew-cli-live-cli-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&fixture_dir).unwrap();

    // SAFETY: `RUNTIME_DIR_LOCK` is held for this whole scenario, so no
    // sibling test in this binary can observe or race the mutation.
    let _env = RUNTIME_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("HEW_RUNTIME_DIR", &fixture_dir);
    }

    // 1. Empty directory: discovery finds nothing (the normal case when no
    //    app is running).
    assert!(
        hew_cli::live::discover().is_empty(),
        "a fresh fixture dir has no candidates"
    );

    // 2. A stale entry (a pid that is not alive) is both excluded from
    //    discovery and deleted from disk on the same pass — §11.2's
    //    validate-then-use contract.
    const DEAD_PID: u32 = 999_999;
    let stale_file = fixture_dir
        .join("hew")
        .join(format!("instance-{DEAD_PID}.json"));
    std::fs::create_dir_all(fixture_dir.join("hew")).unwrap();
    write_instance_file(
        &fixture_dir.join("hew"),
        DEAD_PID,
        "/tmp/nonexistent.sock",
        "dead-tok",
    );
    assert!(hew_cli::live::discover().is_empty());
    assert!(!stale_file.exists(), "the stale discovery file was deleted");

    // 3. Two live candidates (this process's own pid, and a real spawned
    //    child's pid) — discovery finds both, and a live `dispatch` with no
    //    --instance is ambiguous.
    let own_socket = short_socket_path("own");
    let own_server = spawn_attach_gated_fake_instance(own_socket.clone(), "fixture-token");
    write_instance_file(
        &fixture_dir.join("hew"),
        std::process::id(),
        own_socket.to_str().unwrap(),
        "fixture-token",
    );

    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn a real second process to get a second live pid");
    let child_pid = child.id();
    // This socket is never actually dialed in the ambiguous case below —
    // ambiguity is decided from discovery alone, before any connection
    // attempt — so it does not need a listener behind it.
    write_instance_file(
        &fixture_dir.join("hew"),
        child_pid,
        "/tmp/hew-live-cli-test-unused.sock",
        "other-token",
    );

    let found = hew_cli::live::discover();
    assert_eq!(found.len(), 2, "both live pids are discovered");

    let ambiguous = hew_cli::run::dispatch_live(
        "hew.query.scene",
        json!({}),
        &hew_cli::live::LiveOptions {
            launch: false,
            instance: None,
        },
    );
    assert_eq!(
        ambiguous.exit_code, 1,
        "two live candidates with no --instance must refuse to guess"
    );
    assert!(
        ambiguous.response.is_none(),
        "an ambiguous discovery never reaches the transport"
    );

    // 4. `--instance <pid>` disambiguates and the whole round trip
    //    succeeds: discover (via HEW_RUNTIME_DIR) -> connect -> the
    //    mandatory token-bearing hello -> hew.doc.attach -> the forwarded
    //    command -> its reply. The attach-gated fake would answer -32002
    //    for hew.query.scene if `dispatch_live` skipped the attach step
    //    (the regression this whole rewritten fake exists to catch).
    let picked = hew_cli::run::dispatch_live(
        "hew.query.scene",
        json!({}),
        &hew_cli::live::LiveOptions {
            launch: false,
            instance: Some(std::process::id()),
        },
    );
    assert_eq!(picked.exit_code, 0, "response: {:?}", picked.response);
    let response = picked.response.expect("dispatch_live produced a response");
    assert_eq!(response["result"]["tree"], json!([]));

    let seen = own_server
        .join()
        .expect("the fake instance thread completes cleanly");
    assert_eq!(
        seen,
        vec!["hew.doc.attach", "hew.query.scene"],
        "dispatch --live must attach before dispatching the requested command"
    );

    // 5. `hew-cli mcp --live` performs the identical attach step at
    //    construction time — before any `tools/call` — and a forwarded
    //    tool call (here `hew_describe_scene`, which maps to
    //    `hew.query.scene`) succeeds afterward instead of hitting -32002.
    let mcp_socket = short_socket_path("mcp");
    let mcp_server_thread =
        spawn_attach_gated_fake_instance(mcp_socket.clone(), "fixture-token-mcp");
    write_instance_file(
        &fixture_dir.join("hew"),
        std::process::id(),
        mcp_socket.to_str().unwrap(),
        "fixture-token-mcp",
    );
    let mut mcp_server = hew_cli::mcp::McpServer::new_live(&hew_cli::live::LiveOptions {
        launch: false,
        instance: Some(std::process::id()),
    })
    .expect("mcp --live connects and attaches");
    let line = mcp_server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"hew_describe_scene","arguments":{}}}"#,
        )
        .expect("tools/call replies");
    let reply: Value = serde_json::from_str(&line).unwrap();
    let inner: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        inner["result"]["tree"],
        json!([]),
        "the forwarded tool call must succeed, not -32002: {inner}"
    );
    drop(mcp_server); // closes the socket so the fake below sees EOF
    let mcp_seen = mcp_server_thread
        .join()
        .expect("the mcp fake instance thread completes cleanly");
    assert_eq!(
        mcp_seen,
        vec!["hew.doc.attach", "hew.query.scene"],
        "mcp --live must attach at construction, before any forwarded tool call"
    );

    // 6. An attach refusal (the app has no document open at all) fails the
    //    connection cleanly — `dispatch --live` never falls through to
    //    dispatch the caller's actual command against an unattached
    //    session.
    let refusing_socket = short_socket_path("refuse");
    let refusing_server =
        spawn_attach_refusing_fake_instance(refusing_socket.clone(), "refuse-token");
    write_instance_file(
        &fixture_dir.join("hew"),
        child_pid,
        refusing_socket.to_str().unwrap(),
        "refuse-token",
    );
    let refused = hew_cli::run::dispatch_live(
        "hew.query.scene",
        json!({}),
        &hew_cli::live::LiveOptions {
            launch: false,
            instance: Some(child_pid),
        },
    );
    assert_eq!(
        refused.exit_code, 1,
        "an attach refusal must fail the dispatch"
    );
    assert!(
        refused.response.is_none(),
        "an attach refusal must not fall through to the requested command"
    );
    refusing_server
        .join()
        .expect("the refusing fake instance thread completes cleanly");

    let _ = child.kill();
    let _ = child.wait();
    unsafe {
        std::env::remove_var("HEW_RUNTIME_DIR");
    }
    let _ = std::fs::remove_dir_all(&fixture_dir);
    let _ = std::fs::remove_file(&own_socket);
    let _ = std::fs::remove_file(&mcp_socket);
    let _ = std::fs::remove_file(&refusing_socket);
}

/// A live `hew.doc.save`/`export` carrying a path must be written HERE:
/// the app has no filesystem inside its sandbox, so it hands the bytes
/// back and this side writes them. Both live entry points have to do it
/// — `dispatch --live` did and `run --live` did not, so a script asking
/// to save got a bare refusal while the identical one-shot command
/// worked. The fake instance above refuses a forwarded path exactly as
/// the real host does, so a regression here fails loudly.
#[test]
fn a_live_script_writes_the_bytes_a_save_hands_back() {
    let fixture_dir = std::env::temp_dir().join(format!(
        "hew-cli-live-save-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(fixture_dir.join("hew")).unwrap();
    // SAFETY: see `RUNTIME_DIR_LOCK` — held for this whole scenario.
    let _env = RUNTIME_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("HEW_RUNTIME_DIR", &fixture_dir);
    }

    let socket = short_socket_path("live-save");
    let handle = spawn_attach_gated_fake_instance(socket.clone(), "save-token");
    write_instance_file(
        &fixture_dir.join("hew"),
        std::process::id(),
        socket.to_str().unwrap(),
        "save-token",
    );

    let target = fixture_dir.join("written-by-the-client.hew");
    let script = fixture_dir.join("save.jsonl");
    std::fs::write(
        &script,
        format!(
            "{}\n{}\n{}\n",
            json!({"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}),
            json!({"jsonrpc":"2.0","id":1,"method":"hew.doc.attach","params":{}}),
            json!({"jsonrpc":"2.0","id":2,"method":"hew.doc.save",
                   "params":{"path": target.to_str().unwrap()}}),
        ),
    )
    .unwrap();

    let live = hew_cli::live::LiveOptions {
        launch: false,
        instance: None,
    };
    let outcome = hew_cli::run::run_script(&script, None, Some(&live));
    assert_eq!(outcome.exit_code, 0, "the script must succeed, not refuse");
    assert_eq!(
        std::fs::read(&target).expect("the client wrote the file"),
        b"hew!",
        "the bytes the host handed back must land on disk verbatim"
    );

    let seen = handle.join().expect("fake instance thread");
    assert!(
        seen.contains(&"hew.doc.save".to_string()),
        "the save must actually reach the host: {seen:?}"
    );

    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&target);
}
