//! Server-originated events over the live transport (docs/agents/HEW_API.md
//! §4.5, §12): a fake desktop instance writes a notification between two
//! replies, and the client hands back the right reply while keeping the
//! notification for `take_events`; `hew-cli events` prints what arrives and
//! exits 0 when the app closes the connection.
//!
//! Unix-only, like `live_cli.rs`: the fake instance is a `UnixListener`.
#![cfg(unix)]

use api::{Request, RequestId};
use hew_cli::live::{self, Instance, LiveError, LiveSession};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

fn socket_path(tag: &str) -> PathBuf {
    PathBuf::from("/tmp").join(format!("hew-events-{tag}-{}.sock", std::process::id()))
}

fn instance(path: &std::path::Path) -> Instance {
    Instance {
        socket: path.to_str().unwrap().to_string(),
        token: "t".to_string(),
        pid: std::process::id(),
        version: "1.3.0".to_string(),
        file: PathBuf::new(),
    }
}

fn request(id: &str, method: &str, params: Value) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Text(id.to_string())),
        method: method.to_string(),
        params: Some(params),
    }
}

/// A fake instance that answers each frame with `{}` (hello with an app
/// hello), writing `before_reply[i]` notifications ahead of the reply to
/// the i-th frame after hello, then closes once `frames` frames are
/// answered. Returns the methods it saw.
fn fake(
    path: PathBuf,
    frames: usize,
    before_reply: Vec<Vec<Value>>,
    after_last: Vec<Value>,
) -> std::thread::JoinHandle<Vec<String>> {
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind the fake instance");
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        let mut next = || {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read a frame");
            serde_json::from_str::<Value>(line.trim_end()).unwrap()
        };
        let hello = next();
        writeln!(
            writer,
            "{}",
            json!({ "jsonrpc": "2.0", "id": hello["id"], "result": {
                "protocol": 1, "app": { "name": "hew", "version": "1.3.0" },
                "profile": "app", "encoding": "json", "documents": [] } })
        )
        .unwrap();
        let mut seen = Vec::new();
        for i in 0..frames {
            let frame = next();
            seen.push(frame["method"].as_str().unwrap().to_string());
            for event in before_reply.get(i).into_iter().flatten() {
                writeln!(writer, "{event}").unwrap();
            }
            let result = if frame["method"] == "hew.event.subscribe" {
                json!({ "subscribed": frame["params"]["events"], "denied": [] })
            } else {
                json!({})
            };
            writeln!(
                writer,
                "{}",
                json!({ "jsonrpc": "2.0", "id": frame["id"], "result": result })
            )
            .unwrap();
        }
        for event in after_last {
            writeln!(writer, "{event}").unwrap();
        }
        seen
    })
}

fn event(name: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": format!("hew.event.{name}"), "params": params })
}

#[test]
fn a_notification_between_replies_is_kept_aside_not_taken_for_a_reply() {
    let path = socket_path("between");
    let changed = event("document_changed", json!({ "kind": "commit" }));
    let units = event("units_changed", json!({ "units": "mm" }));
    let server = fake(
        path.clone(),
        2,
        vec![vec![changed.clone()], vec![units.clone()]],
        vec![],
    );
    let mut session =
        LiveSession::handshake(instance(&path), live::build_hello_request("t")).expect("handshake");

    let first = session
        .dispatch(request("a", "hew.doc.attach", json!({})))
        .expect("first reply");
    let api::DispatchOutcome::Reply(first) = first else {
        panic!("a reply")
    };
    assert_eq!(first.id, Some(RequestId::Text("a".into())));
    assert_eq!(session.take_events(), vec![changed]);
    assert!(session.take_events().is_empty(), "handed out once");

    let api::DispatchOutcome::Reply(second) = session
        .dispatch(request("b", "hew.query.scene", json!({})))
        .expect("second reply")
    else {
        panic!("a reply")
    };
    assert_eq!(second.id, Some(RequestId::Text("b".into())));
    assert_eq!(session.take_events(), vec![units]);

    // The app hangs up: read_frame says so.
    assert_eq!(
        server.join().unwrap(),
        vec!["hew.doc.attach", "hew.query.scene"]
    );
    assert!(matches!(session.read_frame(None), Err(LiveError::Closed)));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn events_prints_every_frame_and_exits_zero_when_the_app_closes() {
    let path = socket_path("print");
    let early = event("context_changed", json!({ "stack": [] }));
    let later = [
        event("document_changed", json!({ "kind": "commit" })),
        event("document_changed", json!({ "kind": "undo" })),
    ];
    // attach, then subscribe (with a notification ahead of its reply).
    let server = fake(
        path.clone(),
        2,
        vec![vec![], vec![early.clone()]],
        later.to_vec(),
    );
    let mut session =
        LiveSession::handshake(instance(&path), live::build_hello_request("t")).expect("handshake");
    let mut out = Vec::new();
    let code = hew_cli::run::events_session(
        &mut session,
        None,
        Some(vec!["document_changed".into(), "context_changed".into()]),
        &mut out,
    );
    assert_eq!(code, 0);
    assert_eq!(
        server.join().unwrap(),
        vec!["hew.doc.attach", "hew.event.subscribe"]
    );
    let printed: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(printed, vec![early, later[0].clone(), later[1].clone()]);
    let _ = std::fs::remove_file(&path);
}
