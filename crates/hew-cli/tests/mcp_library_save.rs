//! CLI-1: a headless `hew.library.save` over MCP writes the item itself and
//! answers `{path, id}`. The live-only `finish_save` step (which writes an
//! item from bytes a filesystem-less app sent back) must not run for it.
//! Own test binary: it sets `HEW_LIBRARY_DIR` for the whole process.

use hew_cli::mcp::McpServer;
use serde_json::Value;

#[test]
fn headless_library_save_without_return_bytes_succeeds() {
    let dir = std::env::temp_dir().join(format!("hew-cli-mcp-libsave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: this binary holds one test and nothing else reads the variable.
    unsafe {
        std::env::set_var("HEW_LIBRARY_DIR", &dir);
    }
    let mut server = McpServer::new();
    let call = |server: &mut McpServer, commands: Value| -> Value {
        let line = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "hew_transact", "arguments": { "commands": commands } }
        })
        .to_string();
        let reply = server.handle_line(&line).expect("a request gets a reply");
        serde_json::from_str(&reply).unwrap()
    };
    let built = call(
        &mut server,
        serde_json::json!([
            { "method": "hew.sketch.draw_rect", "as": "rect", "params": {
                "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [1.0, 1.0, 0.0]
            } },
            { "method": "hew.solid.extrude", "params": {
                "region": { "$ref": "rect#/region_id" }, "distance": 1.0
            } },
        ]),
    );
    assert!(built.get("error").is_none(), "{built}");
    // The canonical shape: one library command in a one-command transaction.
    let v = call(
        &mut server,
        serde_json::json!([
            { "method": "hew.library.save", "params": {
                "name": "Mcp Save Item", "category": "component"
            } },
        ]),
    );
    assert!(v.get("error").is_none(), "no JSON-RPC error: {v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(!text.contains("returned no item bytes"), "{text}");
    let inner: Value = serde_json::from_str(text).unwrap();
    let save = &inner["result"]["results"][0];
    assert!(
        save["id"].is_string() && save["path"].is_string(),
        "{inner}"
    );
    assert!(dir.join(save["path"].as_str().unwrap()).exists());
    let _ = std::fs::remove_dir_all(&dir);
}
