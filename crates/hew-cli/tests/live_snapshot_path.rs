//! `hew-cli dispatch hew.view.snapshot '{"path":…}' --live` against a stub
//! live host: the client strips `path`, asks the host to render once, and
//! writes the host's own bytes (and the companion pass files) itself.
//!
//! Unix-only, like `live_cli.rs`: the stub is a `UnixListener`.
#![cfg(unix)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;

/// `HEW_RUNTIME_DIR` is process-wide; this binary has one test that sets it.
#[test]
fn live_snapshot_with_a_path_is_written_once_from_the_hosts_bytes() {
    let dir = std::env::temp_dir().join(format!("hew-cli-live-snap-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("hew")).unwrap();
    // SAFETY: the only test in this binary.
    unsafe {
        std::env::set_var("HEW_RUNTIME_DIR", &dir);
    }
    let socket = std::path::PathBuf::from("/tmp")
        .join(format!("hew-live-snap-test-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).unwrap();
    let pid = std::process::id();
    std::fs::write(
        dir.join("hew").join(format!("instance-{pid}.json")),
        serde_json::to_vec(&json!({
            "socket": socket.to_str().unwrap(), "token": "t", "pid": pid, "version": "0.5.0"
        }))
        .unwrap(),
    )
    .unwrap();

    // Every `hew.view.snapshot` frame the stub receives, params included.
    let host = std::thread::spawn(move || {
        let mut snapshots: Vec<Value> = Vec::new();
        // Two connections: the good write, then the failing one.
        for _ in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let frame: Value = serde_json::from_str(line.trim_end()).unwrap();
                let result = match frame["method"].as_str().unwrap_or("") {
                    "hew.meta.hello" => json!({
                        "protocol": 1, "app": { "name": "hew", "version": "0.5.0" },
                        "profile": "app", "encoding": "json", "documents": [],
                    }),
                    "hew.view.snapshot" => {
                        snapshots.push(frame["params"].clone());
                        // "host!" / "ids" / "depth" in base64.
                        json!({
                            "png_base64": "aG9zdCE=", "width": 64, "height": 48,
                            "id_buffer_base64": "aWRz", "id_palette": ["obj_0"],
                            "depth_png_base64": "ZGVwdGg=",
                            "depth_near": 1.0, "depth_far": 2.0, "depth_bits": 8,
                        })
                    }
                    _ => json!({}),
                };
                writeln!(
                    writer,
                    "{}",
                    json!({ "jsonrpc": "2.0", "id": frame["id"], "result": result })
                )
                .unwrap();
            }
        }
        snapshots
    });

    let target = dir.join("shot.png");
    let live = hew_cli::live::LiveOptions {
        launch: false,
        instance: None,
        document: None,
    };
    let outcome = hew_cli::run::dispatch_live(
        "hew.view.snapshot",
        json!({ "path": target.to_str().unwrap(), "include_ids": true, "include_depth": true }),
        &live,
    );
    assert_eq!(outcome.exit_code, 0, "{:?}", outcome.response);

    assert_eq!(std::fs::read(&target).unwrap(), b"host!");
    assert_eq!(
        std::fs::read(format!("{}.ids.bin", target.display())).unwrap(),
        b"ids"
    );
    assert_eq!(
        std::fs::read(format!("{}.depth.png", target.display())).unwrap(),
        b"depth"
    );

    // A path whose directory does not exist: the write fails, exit 1, and
    // the reply is the canonical save_failed refusal.
    let failed = hew_cli::run::dispatch_live(
        "hew.view.snapshot",
        json!({ "path": dir.join("no/such/dir/shot.png").to_str().unwrap() }),
        &live,
    );
    assert_eq!(failed.exit_code, 1, "{:?}", failed.response);
    let reply = failed.response.unwrap();
    assert_eq!(reply["error"]["code"], -32000, "{reply}");
    assert_eq!(reply["error"]["data"]["refusal"], "save_failed", "{reply}");

    let snapshots = host.join().unwrap();
    assert_eq!(
        snapshots.len(),
        2,
        "exactly one render request per call: {snapshots:?}"
    );
    assert!(
        snapshots[0].get("path").is_none(),
        "the request carries no path: {:?}",
        snapshots[0]
    );

    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_dir_all(&dir);
}
