//! The Preview wall at the `--live` socket (docs/dev/DEVELOPMENT.md
//! "Preview features", "Code loading"): what a production `hew-cli` and a
//! preview `hew-cli` each send a running app, paired with a fake app on a
//! real local socket that records every frame it is sent.
//!
//! In a production build a method its own registry marks as a Preview
//! command is refused `preview_only` before anything is sent, for
//! `dispatch`, `run` and `mcp`, bare and inside `hew.doc.transact`; a method
//! the registry does not know at all is forwarded (a newer app may serve
//! it). In a preview build such methods are forwarded, and an app that
//! answers `preview_only` (a production app) is reported as the refusal it
//! is. In every build the MCP server refuses `hew.plugin.dev` to an agent.
//!
//! `HEW_RUNTIME_DIR` is process-wide, so every scenario takes one lock.
#![cfg(unix)]

use hew_cli::live::LiveOptions;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

static RUNTIME_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the fake app answers for a method it was sent.
#[derive(Clone, Copy, PartialEq)]
enum App {
    /// A production app: it refuses every Preview command `preview_only`.
    Production,
    /// A preview app that answers the plugin commands.
    Preview,
    /// A preview app that closes the connection on `hew.plugin.dev`
    /// without answering it.
    #[cfg_attr(not(hew_preview), allow(dead_code))]
    ClosesOnDev,
    /// A preview app whose list says no request is waiting, as after a
    /// dismiss or an expiry.
    #[cfg_attr(not(hew_preview), allow(dead_code))]
    NoRequestWaiting,
    /// A preview app whose list says a request is waiting.
    #[cfg_attr(not(hew_preview), allow(dead_code))]
    RequestWaiting,
}

/// A command the registry marks that is not a plugin command, so the
/// pre-forward refusal is shown to be about the registry and not a name.
fn another_preview_command() -> String {
    api::preview::features()
        .iter()
        .flat_map(|f| f.api_commands.iter())
        .find(|c| !c.starts_with("hew.plugin."))
        .expect("the registry marks some other Preview command")
        .clone()
}

fn socket_path(tag: &str) -> PathBuf {
    PathBuf::from("/tmp").join(format!("hew-live-wall-{tag}-{}.sock", std::process::id()))
}

fn answer_for(app: App, method: &str, params: &Value) -> Value {
    let preview_only = || {
        json!({ "code": -32000, "message": "refused",
                "data": { "refusal": "preview_only", "failed_index": 0,
                          "failed_method": method, "detail": {},
                          "explanation": "preview feature" } })
    };
    let marked = api::preview::features()
        .iter()
        .any(|f| f.api_commands.iter().any(|c| c == method));
    match method {
        "hew.doc.attach" | "hew.event.subscribe" => json!({ "result": {} }),
        "hew.meta.capabilities" => json!({ "result": { "commands": [] } }),
        "hew.plugin.list" if app != App::Production => {
            let mut result = json!({ "plugins": [], "developer_mode": true });
            match app {
                App::NoRequestWaiting => result["request_waiting"] = json!(false),
                App::RequestWaiting => result["request_waiting"] = json!(true),
                _ => {}
            }
            json!({ "result": result })
        }
        "hew.plugin.dev" if app != App::Production => {
            assert!(params["path"].is_string());
            json!({ "result": { "status": "awaiting_user" } })
        }
        _ if marked && app == App::Production => json!({ "error": preview_only() }),
        "hew.doc.transact" => {
            // A production app refuses the whole envelope at the first
            // marked command, as the dispatcher's static pass does.
            let commands = params["commands"].as_array().cloned().unwrap_or_default();
            let hit = commands.iter().enumerate().find(|(_, c)| {
                let m = c["method"].as_str().unwrap_or("");
                api::preview::features()
                    .iter()
                    .any(|f| f.api_commands.iter().any(|n| n == m))
            });
            match (hit, app) {
                (Some(_), App::Production) => json!({ "error": preview_only() }),
                _ => json!({ "result": { "label": "t", "results": [{}] } }),
            }
        }
        _ => json!({ "result": { "forwarded": true } }),
    }
}

/// One accepted connection: hello, then every frame is recorded (a
/// `hew.doc.transact`'s inner methods as `in-transact:<method>`) and
/// answered by [`answer_for`] until the client disconnects.
fn spawn_fake_app(
    socket: PathBuf,
    token: &'static str,
    app: App,
) -> std::thread::JoinHandle<Vec<String>> {
    let listener = UnixListener::bind(&socket).expect("bind the fake app socket");
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept the one connection");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        let mut seen = Vec::new();
        let mut line = String::new();
        reader.read_line(&mut line).expect("read the hello");
        let hello: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(hello["params"]["token"], token);
        let reply = json!({ "jsonrpc": "2.0", "id": hello["id"], "result": {
            "protocol": 1, "app": { "name": "hew", "version": "0.5.0" },
            "profile": "app", "encoding": "json", "documents": [] } });
        writeln!(writer, "{reply}").unwrap();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).expect("read a frame") == 0 {
                break;
            }
            let frame: Value = serde_json::from_str(line.trim_end()).unwrap();
            let method = frame["method"].as_str().unwrap_or("").to_string();
            seen.push(method.clone());
            if app == App::ClosesOnDev && method == "hew.plugin.dev" {
                break;
            }
            if method == "hew.doc.transact" {
                for inner in frame["params"]["commands"].as_array().into_iter().flatten() {
                    seen.push(format!(
                        "in-transact:{}",
                        inner["method"].as_str().unwrap_or("")
                    ));
                }
            }
            let mut reply = answer_for(app, &method, &frame["params"]);
            reply["jsonrpc"] = json!("2.0");
            reply["id"] = frame["id"].clone();
            writeln!(writer, "{reply}").unwrap();
        }
        seen
    })
}

/// Points discovery at a fresh runtime directory holding one fake app.
/// The returned guard keeps the process-wide lock.
struct Scenario {
    _lock: std::sync::MutexGuard<'static, ()>,
    dir: PathBuf,
    socket: PathBuf,
    server: Option<std::thread::JoinHandle<Vec<String>>>,
}

impl Scenario {
    fn new(tag: &'static str, app: App) -> Scenario {
        let lock = RUNTIME_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "hew-cli-live-wall-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("hew")).unwrap();
        // SAFETY: the lock held in `_lock` serializes every use of the
        // variable in this test binary.
        unsafe {
            std::env::set_var("HEW_RUNTIME_DIR", &dir);
        }
        let socket = socket_path(tag);
        let _ = std::fs::remove_file(&socket);
        let server = spawn_fake_app(socket.clone(), "wall-token", app);
        let body = json!({ "socket": socket.to_str().unwrap(), "token": "wall-token",
                           "pid": std::process::id(), "version": "0.5.0" });
        std::fs::write(
            dir.join("hew")
                .join(format!("instance-{}.json", std::process::id())),
            serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();
        Scenario {
            _lock: lock,
            dir,
            socket,
            server: Some(server),
        }
    }

    fn opts(&self) -> LiveOptions {
        LiveOptions {
            launch: false,
            instance: Some(std::process::id()),
            document: None,
        }
    }

    /// Every frame the fake app saw after hello, once the client is done.
    fn seen(mut self) -> Vec<String> {
        let seen = self
            .server
            .take()
            .unwrap()
            .join()
            .expect("the fake app thread");
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_dir_all(&self.dir);
        seen
    }
}

fn refusal_of(response: &Value) -> Option<&str> {
    response
        .pointer("/error/data/refusal")
        .and_then(Value::as_str)
}

fn reached(seen: &[String], method: &str) -> bool {
    seen.iter()
        .any(|m| m == method || m.ends_with(&format!(":{method}")))
}

/// One `dispatch --live` of `method` against a fresh fake app: the result
/// and every frame the app saw.
fn dispatch_once(
    tag: &'static str,
    app: App,
    method: &str,
    params: Value,
) -> (hew_cli::run::DispatchResult, Vec<String>) {
    let scenario = Scenario::new(tag, app);
    let outcome = hew_cli::run::dispatch_live(method, params, &scenario.opts());
    (outcome, scenario.seen())
}

/// `dispatch --live` of a Preview command: refused before sending in a
/// production build, forwarded in a preview one; an unknown name is
/// forwarded in both.
#[test]
fn dispatch_live_of_a_preview_command_follows_the_build() {
    let preview = api::preview::PREVIEW_BUILD;
    let other = another_preview_command();
    for app in [App::Production, App::Preview] {
        for (n, method) in [
            "hew.plugin.dev",
            "hew.plugin.list",
            other.as_str(),
            "hew.future.thing",
        ]
        .into_iter()
        .enumerate()
        {
            let tag = ["d0", "d1", "d2", "d3"][n];
            let params = if method == "hew.plugin.dev" {
                json!({ "path": "/tmp/p" })
            } else {
                json!({})
            };
            let (outcome, seen) = dispatch_once(tag, app, method, params);
            let response = outcome.response.as_ref().expect("an answer");
            if method == "hew.future.thing" {
                // Unknown to this registry: forwarded in every build.
                assert!(
                    reached(&seen, method),
                    "{method} was not forwarded: {seen:?}"
                );
                assert_eq!(outcome.exit_code, 0, "{method}: {response}");
            } else if preview {
                assert!(
                    reached(&seen, method),
                    "{method} was not forwarded: {seen:?}"
                );
                if app == App::Production {
                    // A production app refuses; the CLI reports it as is.
                    assert_eq!(outcome.exit_code, 1, "{method}: {response}");
                    assert_eq!(refusal_of(response), Some("preview_only"), "{method}");
                } else if method.starts_with("hew.plugin.") {
                    assert_eq!(outcome.exit_code, 0, "{method}: {response}");
                }
            } else {
                assert!(
                    !reached(&seen, method),
                    "a production build sent {method}: {seen:?}"
                );
                assert_eq!(outcome.exit_code, 1, "{method}: {response}");
                assert_eq!(refusal_of(response), Some("preview_only"), "{method}");
                assert_eq!(response["error"]["data"]["failed_method"], method);
            }
        }
    }
}

/// A Preview command inside a transaction is held to the same rule, and
/// the refusal names the command's place in it.
#[test]
fn a_preview_command_in_a_live_transaction_is_not_sent_by_a_production_build() {
    let preview = api::preview::PREVIEW_BUILD;
    let scenario = Scenario::new("transact", App::Preview);
    let opts = scenario.opts();
    let params = json!({ "commands": [
        { "method": "hew.view.zoom_extents", "params": {} },
        { "method": "hew.plugin.dev", "params": { "path": "/tmp/p" } },
    ] });
    let outcome = hew_cli::run::dispatch_live("hew.doc.transact", params, &opts);
    let seen = scenario.seen();
    if preview {
        assert!(reached(&seen, "hew.plugin.dev"), "{seen:?}");
    } else {
        let response = outcome.response.expect("an answer");
        assert_eq!(outcome.exit_code, 1, "{response}");
        assert_eq!(refusal_of(&response), Some("preview_only"), "{response}");
        assert_eq!(response["error"]["data"]["failed_index"], 1);
        assert_eq!(response["error"]["data"]["failed_method"], "hew.plugin.dev");
        assert!(
            !seen.iter().any(|m| m == "hew.doc.transact"),
            "the envelope was sent: {seen:?}"
        );
    }
}

/// `run --live` goes through the same place.
#[test]
fn run_live_of_a_preview_command_follows_the_build() {
    let preview = api::preview::PREVIEW_BUILD;
    let scenario = Scenario::new("run", App::Production);
    let script = scenario.dir.join("plugin.jsonl");
    std::fs::write(
        &script,
        format!(
            "{}\n{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}),
            json!({"jsonrpc":"2.0","id":2,"method":"hew.plugin.list","params":{}}),
        ),
    )
    .unwrap();
    let outcome = hew_cli::run::run_script(&script, None, Some(&scenario.opts()));
    let seen = scenario.seen();
    assert_eq!(outcome.exit_code, 1, "{:?}", outcome.responses);
    assert_eq!(
        refusal_of(outcome.responses.last().expect("an answer")),
        Some("preview_only")
    );
    assert_eq!(reached(&seen, "hew.plugin.list"), preview, "{seen:?}");
}

/// The MCP server: `hew.plugin.dev` is refused to an agent in every build
/// and never sent; `hew.plugin.list` passes the agent rule, so a preview
/// build sends it, and a production build refuses it `preview_only` (the
/// registry's answer wins) without sending it.
#[test]
fn live_mcp_refuses_plugin_management_in_every_build() {
    let preview = api::preview::PREVIEW_BUILD;
    let scenario = Scenario::new("mcp", App::Preview);
    let mut server = hew_cli::mcp::McpServer::new_live(&scenario.opts()).expect("a live server");
    let mut call = |id: u32, name: &str, arguments: Value| -> Value {
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
                            "params": { "name": name, "arguments": arguments } });
        serde_json::from_str(&server.handle_line(&frame.to_string()).expect("a reply")).unwrap()
    };
    let inner = |frame: &Value| -> Value {
        serde_json::from_str(
            frame["result"]["content"][0]["text"]
                .as_str()
                .expect("text"),
        )
        .expect("tool content is JSON")
    };
    let dev = json!({ "commands": [
        { "method": "hew.plugin.dev", "params": { "path": "/tmp/p" } } ] });
    let frame = call(1, "hew_transact", dev);
    if preview {
        // The agent rule: a JSON-RPC denial, nothing sent.
        assert_eq!(frame["error"]["code"], -32602, "{frame}");
        assert!(
            frame["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not available to agents")
        );
    } else {
        // The registry's own answer for a marked name in a production build.
        assert_eq!(refusal_of(&inner(&frame)), Some("preview_only"), "{frame}");
    }
    let list = json!({ "commands": [ { "method": "hew.plugin.list", "params": {} } ] });
    let frame = call(2, "hew_transact", list);
    assert!(frame.get("error").is_none(), "{frame}");
    let answer = inner(&frame);
    if preview {
        assert!(answer.get("error").is_none(), "{answer}");
    } else {
        assert_eq!(refusal_of(&answer), Some("preview_only"), "{answer}");
    }
    let seen = {
        drop(server);
        scenario.seen()
    };
    assert!(!reached(&seen, "hew.plugin.dev"), "dev was sent: {seen:?}");
    assert_eq!(reached(&seen, "hew.plugin.list"), preview, "{seen:?}");
}

/// A preview CLI talking to a production app: the refusal is the app's,
/// and `plugin dev` says so in plain words.
#[cfg(hew_preview)]
#[test]
fn plugin_dev_against_a_production_app_says_it_has_no_plugin_support() {
    let scenario = Scenario::new("p-prod", App::Production);
    let mut session = hew_cli::live::connect_live(
        &scenario.opts(),
        hew_cli::live::build_hello_request("hew-cli:test"),
    )
    .expect("connect");
    let mut out = Vec::new();
    let code = hew_cli::plugin::dev_session(
        &mut session,
        "/tmp/p",
        &hew_cli::plugin::DevOptions {
            once: true,
            ..Default::default()
        },
        &mut out,
    );
    drop(session);
    let _ = scenario.seen();
    assert_eq!(code, 1);
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("without plugin support"),
        "plain words"
    );
}

/// `plugin dev --once` against a preview app: the request is sent, the
/// waiting line is printed, and the command stops with 0.
#[cfg(hew_preview)]
#[test]
fn plugin_dev_once_sends_the_request_and_stops() {
    let scenario = Scenario::new("p-once", App::Preview);
    let mut session = hew_cli::live::connect_live(
        &scenario.opts(),
        hew_cli::live::build_hello_request("hew-cli:test"),
    )
    .expect("connect");
    let mut out = Vec::new();
    let code = hew_cli::plugin::dev_session(
        &mut session,
        "/tmp/p",
        &hew_cli::plugin::DevOptions {
            once: true,
            ..Default::default()
        },
        &mut out,
    );
    drop(session);
    let seen = scenario.seen();
    assert_eq!(code, 0);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Asked Hew to open this folder:\n  /tmp/p\nWaiting for you in Hew: choose the folder and read what the plugin asks for.\n"
    );
    assert_eq!(seen, ["hew.plugin.dev"], "a script's one request only");
}

/// An app that closes before it answers `hew.plugin.dev` is a failure the
/// command reports with exit 1, not a normal end.
#[cfg(hew_preview)]
#[test]
fn plugin_dev_exits_one_when_the_app_closes_before_answering() {
    let scenario = Scenario::new("closes", App::ClosesOnDev);
    let opts = scenario.opts();
    let folder = std::env::temp_dir();
    let code = hew_cli::plugin::dev_live(
        &folder,
        &opts,
        &hew_cli::plugin::DevOptions {
            once: true,
            ..Default::default()
        },
    );
    let seen = scenario.seen();
    assert!(reached(&seen, "hew.plugin.dev"), "{seen:?}");
    assert_eq!(code, 1);
}

/// Runs `plugin dev` against a fake app for `polls` looks at the list;
/// the exit code and what it printed.
#[cfg(hew_preview)]
fn dev_for(tag: &'static str, app: App, polls: usize) -> (i32, String) {
    let scenario = Scenario::new(tag, app);
    let mut session = hew_cli::live::connect_live(
        &scenario.opts(),
        hew_cli::live::build_hello_request("hew-cli:test"),
    )
    .expect("connect");
    let mut out = Vec::new();
    let code = hew_cli::plugin::dev_session(
        &mut session,
        "/tmp/p",
        &hew_cli::plugin::DevOptions {
            once: false,
            interval: std::time::Duration::from_millis(5),
            max_polls: Some(polls),
        },
        &mut out,
    );
    drop(session);
    let _ = scenario.seen();
    (code, String::from_utf8(out).unwrap())
}

/// A request that is no longer waiting, with nothing installed, is a
/// failure that says so, instead of a silent wait.
#[cfg(hew_preview)]
#[test]
fn plugin_dev_fails_when_the_request_is_no_longer_waiting() {
    let (code, out) = dev_for("gone", App::NoRequestWaiting, 5);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains(
            "The request is no longer waiting in Hew: it was dismissed or declined, or it ran out of time.\n"
        ),
        "{out}"
    );
}

/// While the app says it waits, and from an app that does not say, the
/// command keeps watching.
#[cfg(hew_preview)]
#[test]
fn plugin_dev_keeps_watching_while_the_request_waits_or_the_app_does_not_say() {
    for (tag, app) in [
        ("wait-yes", App::RequestWaiting),
        ("wait-unsaid", App::Preview),
    ] {
        let (code, out) = dev_for(tag, app, 4);
        assert_eq!(code, 0, "{tag}: {out}");
        assert!(!out.contains("no longer waiting"), "{tag}: {out}");
    }
}
