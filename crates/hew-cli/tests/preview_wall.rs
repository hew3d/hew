//! The Preview wall on `hew-cli`'s own surfaces (docs/dev/DEVELOPMENT.md
//! "Preview features"): the MCP server (its tool list, `hew_capabilities`,
//! `hew_transact` and `hew_query`), `hew-cli run`, and `hew-cli dispatch`.
//! Every test walks `preview-features.json` through `api::preview`; none
//! names a feature. A production build (plain `cargo test -p hew-cli`)
//! refuses every preview command `preview_only` and lists none; a preview
//! build (`HEW_PREVIEW=1`) serves them. `--live` frames are answered by the
//! app's own dispatcher, which `crates/wasm-api` tests.

use hew_cli::mcp::McpServer;
use serde_json::{Value, json};
use std::path::PathBuf;

fn gated_commands() -> Vec<(String, String)> {
    api::preview::features()
        .iter()
        .flat_map(|f| f.api_commands.iter().map(|c| (c.clone(), f.id.clone())))
        .collect()
}

/// One MCP request through the server; the JSON-RPC reply.
fn mcp(server: &mut McpServer, method: &str, params: Value) -> Value {
    let line = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let reply = server
        .handle_line(&line.to_string())
        .expect("a request is answered");
    serde_json::from_str(&reply).expect("the reply is JSON")
}

/// A `tools/call`'s inner Hew response (the tool's text content), or the
/// JSON-RPC error when the tool itself refused the call.
fn tool(server: &mut McpServer, name: &str, arguments: Value) -> Value {
    let reply = mcp(
        server,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    );
    match reply
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
    {
        Some(text) => serde_json::from_str(text).expect("tool content is JSON"),
        None => reply,
    }
}

fn refusal_name(response: &Value) -> Option<&str> {
    response
        .pointer("/error/data/refusal")
        .and_then(Value::as_str)
}

fn scratch(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("hew-cli-preview-wall-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// No MCP tool definition names a preview command (the tools are generic;
/// their prose must not advertise one either).
#[test]
fn no_mcp_tool_definition_names_a_preview_command() {
    let mut server = McpServer::new();
    let tools = mcp(&mut server, "tools/list", json!({})).to_string();
    let initialize = mcp(&mut server, "initialize", json!({})).to_string();
    for (command, _) in gated_commands() {
        assert!(!tools.contains(&command), "tools/list names {command}");
        assert!(!initialize.contains(&command), "initialize names {command}");
    }
}

/// `hew_capabilities` lists a preview command only in a preview build;
/// `hew_transact` refuses each `preview_only` in a production build and
/// runs it in a preview one; `hew_query` never admits one it cannot list.
/// (`cfg(hew_preview)` is `crates/api`'s, so this crate asks it.)
#[test]
fn mcp_lists_and_runs_preview_commands_only_in_a_preview_build() {
    let preview = api::preview::PREVIEW_BUILD;
    let mut server = McpServer::new();
    let caps = tool(&mut server, "hew_capabilities", json!({}));
    let listed: Vec<&str> = caps["result"]["commands"]
        .as_array()
        .expect("capabilities lists commands")
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(!listed.is_empty());
    for (command, feature) in gated_commands() {
        assert_eq!(
            listed.contains(&command.as_str()),
            preview,
            "{command} listed in a {} build",
            if preview { "preview" } else { "production" }
        );
        let r = tool(
            &mut server,
            "hew_transact",
            json!({ "commands": [{ "method": command, "params": {} }] }),
        );
        if preview {
            assert_ne!(refusal_name(&r), Some("preview_only"), "{command}: {r}");
        } else {
            assert_eq!(refusal_name(&r), Some("preview_only"), "{command}: {r}");
            assert_eq!(r["error"]["data"]["detail"]["feature"], feature.as_str());
            let reply = mcp(
                &mut server,
                "tools/call",
                json!({ "name": "hew_query", "arguments": { "method": command } }),
            );
            assert!(
                reply.get("result").is_none(),
                "{command} ran through hew_query: {reply}"
            );
            assert_eq!(reply["error"]["code"], -32602, "{command}: {reply}");
        }
    }
}

/// `hew-cli run` and `hew-cli dispatch` answer the dispatcher's verdict.
#[test]
fn run_and_dispatch_answer_the_wall() {
    let dir = scratch("run");
    let model = dir.join("box.hew");
    let base = [
        json!({"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}),
        json!({"jsonrpc":"2.0","id":2,"method":"hew.doc.new","params":{}}),
    ];
    let script = dir.join("base.jsonl");
    std::fs::write(
        &script,
        base.iter()
            .map(|f| f.to_string() + "\n")
            .collect::<String>(),
    )
    .expect("write script");
    let outcome = hew_cli::run::run_script(&script, Some(&model), None);
    assert_eq!(outcome.exit_code, 0, "the base script runs");

    for (command, _) in gated_commands() {
        let script = dir.join("gated.jsonl");
        let frames = [
            base[0].clone(),
            base[1].clone(),
            json!({"jsonrpc":"2.0","id":3,"method":command,"params":{}}),
        ];
        std::fs::write(
            &script,
            frames
                .iter()
                .map(|f| f.to_string() + "\n")
                .collect::<String>(),
        )
        .expect("write script");
        let outcome = hew_cli::run::run_script(&script, None, None);
        let last = outcome.responses.last().expect("a reply").clone();
        let dispatched = hew_cli::run::dispatch_file(&model, &command, json!({}));
        let one_shot = dispatched.response.expect("a reply");
        if api::preview::PREVIEW_BUILD {
            assert_ne!(refusal_name(&last), Some("preview_only"), "run {command}");
            assert_ne!(
                refusal_name(&one_shot),
                Some("preview_only"),
                "dispatch {command}"
            );
        } else {
            assert_eq!(outcome.exit_code, 1, "run {command}");
            assert_eq!(
                refusal_name(&last),
                Some("preview_only"),
                "run {command}: {last}"
            );
            assert_ne!(dispatched.exit_code, 0, "dispatch {command}");
            assert_eq!(
                refusal_name(&one_shot),
                Some("preview_only"),
                "dispatch {command}: {one_shot}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
