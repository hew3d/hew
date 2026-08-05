//! MCP-frame fuzz (suite 3 of the API dispatch fuzz harness —
//! `crates/api/tests/dispatch_fuzz.rs`'s header explains the overall
//! three-suite shape; this file is the MCP-transport-shaped third leg,
//! against a different crate and a different surface: raw
//! newline-delimited JSON-RPC TEXT, not an already-parsed `Request`).
//!
//! `McpServer::handle_line` (`crates/hew-cli/src/mcp.rs`) is a pure state
//! transition — one line in, at most one line out — driven directly here,
//! spawn-free, exactly like `cli.rs`'s own convention. No `proptest` dev
//! dependency exists in this crate and none is added for one file: the
//! hostile-input space this layer needs to cover (malformed JSON, a
//! truncated frame, a structurally-valid-but-wrong-shaped frame) is small
//! and finite enough that a deterministic generator — a seeded xorshift64
//! producing garbage strings, plus truncations of real frames at every
//! character boundary, plus a hand-picked table of wrong-shaped-but-valid
//! JSON — gives the same "never panics, never corrupts state" assurance
//! `dispatch_fuzz.rs` gives one layer down, without pulling in machinery
//! this crate doesn't otherwise need. A defect found here would fail the
//! test outright (a panic simply fails a plain `#[test]`, same posture as
//! proptest's own "the test surviving is the assertion" — no
//! `catch_unwind` needed).
//!
//! Two things are asserted over every hostile line: `handle_line` never
//! panics, and whatever it returns (if anything) is well-formed JSON-RPC
//! (exactly one of `result`/`error`, and an `error.code` drawn from the
//! small set this transport layer can itself produce). Then, after every
//! hostile line has been fed to ONE long-lived server, a real
//! `initialize` -> `tools/list` -> `tools/call` sequence must still work
//! exactly as `mcp.rs`'s own unit tests expect — proving the hostile
//! input never corrupted the server's `Connection`/`Document` state.

use hew_cli::mcp::McpServer;
use serde_json::Value;

/// A tiny deterministic PRNG (xorshift64) — no `rand` dependency for one
/// fuzz file; determinism also means a failure here always reproduces
/// without needing a saved seed file (unlike the proptest suites, this
/// generator has no shrinking, so a failure is reported with its raw
/// input directly).
struct Xorshift64(u64);

impl Xorshift64 {
    fn new(seed: u64) -> Xorshift64 {
        Xorshift64(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_range(&mut self, bound: usize) -> usize {
        (self.next_u64() as usize) % bound.max(1)
    }
}

/// A garbage-string alphabet biased toward JSON's own syntax characters
/// (braces, brackets, quotes, colons, commas, backslashes) plus a few
/// control/unicode characters — the shapes most likely to trip up a
/// hand-rolled line-oriented JSON-RPC reader, per the same "hostile but
/// structurally near-miss" idea `dispatch_fuzz.rs`'s method-name fuzzing
/// uses one layer down.
const GARBAGE_ALPHABET: &[char] = &[
    '{', '}', '[', ']', ':', ',', '"', '\\', 'a', 'b', '0', '1', '-', '.', 'n', 'u', 'l', 't', 'r',
    'e', ' ', '\t', '\u{0000}', '\u{fffd}', '💥', '\u{202e}', '\'', '/', '*',
];

fn random_garbage_line(rng: &mut Xorshift64, max_len: usize) -> String {
    let len = rng.next_range(max_len + 1);
    (0..len)
        .map(|_| GARBAGE_ALPHABET[rng.next_range(GARBAGE_ALPHABET.len())])
        .collect()
}

/// A handful of real, valid frames this server actually accepts —
/// truncated at every character boundary below to fuzz "truncated
/// mid-frame" without ever needing a second, larger PRNG-driven corpus.
fn representative_valid_frames() -> Vec<String> {
    vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"hew_transact","arguments":{"commands":[{"method":"hew.sketch.draw_rect","as":"r","params":{"plane":{"ground":true},"corner_a":[0.0,0.0,0.0],"corner_b":[1.0,1.0,0.0]}}]}}}"#.to_string(),
    ]
}

/// Structurally VALID JSON that is the wrong shape for anything this
/// server expects: non-object frames, missing/mistyped fields, and
/// deliberately oversized structures.
fn wrong_shape_valid_json() -> Vec<String> {
    vec![
        "null".to_string(),
        "true".to_string(),
        "42".to_string(),
        "3.14159".to_string(),
        r#""just a bare string, not a frame at all""#.to_string(),
        "[1,2,3]".to_string(),
        "{}".to_string(),
        r#"{"jsonrpc":"2.0"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":123}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":null}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":"not an object"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":123}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":[1,2,3]}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":123}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":null}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"hew_query","arguments":"nope"}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"hew_query","arguments":{"method":123}}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"unknown_tool_xyz"}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":{"nested":"object","as":"id"},"method":"tools/list"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":[1,2,3],"method":"tools/list"}"#.to_string(),
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"hew_transact","arguments":{{"commands":"{}"}}}}}}"#,
            "x".repeat(4096)
        ),
        // Deeply nested arrays — cheap to build, mean to parse.
        format!(
            "{}{}{}",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"hew_query","arguments":{"method":"hew.query.entity","params":"#,
            "[".repeat(200) + &"]".repeat(200),
            "}}}",
        ),
    ]
}

/// The KNOWN error codes this transport layer's own `JsonRpcErr` (and its
/// one hardcoded internal-error site) can produce — `mcp.rs`'s module
/// doc comment: MCP-layer errors are distinct from a `hew.*` refusal,
/// which never surfaces as a top-level JSON-RPC error (`tools/call`
/// always answers `Ok`, embedding the refusal as tool-result content per
/// docs/HEW_API.md §13 — "refusals are answers").
const KNOWN_MCP_ERROR_CODES: [i64; 4] = [-32700, -32601, -32602, -32603];

/// Feeds one hostile line to `server` and checks it never panics and,
/// if it replies at all, replies with well-formed JSON-RPC.
fn drive_hostile_line(server: &mut McpServer, line: &str) {
    let Some(reply) = server.handle_line(line) else {
        return; // notification-shaped or empty — no reply is legal.
    };
    let v: Value = serde_json::from_str(&reply).unwrap_or_else(|e| {
        panic!("handle_line produced non-JSON output for hostile input {line:?}: {reply:?} ({e})")
    });
    let has_result = v.get("result").is_some();
    let has_error = v.get("error").is_some();
    assert!(
        has_result != has_error,
        "reply must carry exactly one of result/error for hostile input {line:?}, got: {v}"
    );
    if let Some(error) = v.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| panic!("error object lacks an integer code: {error}"));
        assert!(
            KNOWN_MCP_ERROR_CODES.contains(&code),
            "error code {code} is outside this transport's known set for hostile input {line:?}"
        );
    }
}

/// After `count` hostile lines, the server must still behave exactly
/// like a fresh one for the real handshake sequence — proving no hostile
/// line left the `Connection`/`Document` behind it corrupted.
fn assert_server_still_works(server: &mut McpServer) {
    let line = server
        .handle_line(r#"{"jsonrpc":"2.0","id":9001,"method":"initialize","params":{}}"#)
        .expect("initialize replies after hostile input");
    let v: Value = serde_json::from_str(&line).expect("initialize reply is valid JSON");
    assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
    assert!(
        v.get("error").is_none(),
        "initialize must still succeed: {v}"
    );

    let line = server
        .handle_line(r#"{"jsonrpc":"2.0","id":9002,"method":"tools/list","params":{}}"#)
        .expect("tools/list replies after hostile input");
    let v: Value = serde_json::from_str(&line).expect("tools/list reply is valid JSON");
    let tools = v["result"]["tools"]
        .as_array()
        .expect("tools/list still returns a tool array");
    assert_eq!(
        tools.len(),
        5,
        "the headless-core tool inventory is unchanged after hostile input"
    );

    let line = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":9003,"method":"tools/call","params":{"name":"hew_capabilities","arguments":{}}}"#,
        )
        .expect("tools/call replies after hostile input");
    let v: Value = serde_json::from_str(&line).expect("tools/call reply is valid JSON");
    let text = v["result"]["content"][0]["text"]
        .as_str()
        .expect("hew_capabilities still returns tool-result text");
    let inner: Value = serde_json::from_str(text).expect("tool-result text is valid JSON");
    assert!(
        inner["result"]["commands"].as_array().unwrap().len() >= 50,
        "the dispatched hew.meta.capabilities call still answers normally: {inner}"
    );
}

#[test]
fn random_garbage_lines_never_panic_or_desync_the_server() {
    let mut server = McpServer::new();
    let mut rng = Xorshift64::new(0x00C0_FFEE_1234_5678);
    for _ in 0..600 {
        let line = random_garbage_line(&mut rng, 80);
        drive_hostile_line(&mut server, &line);
    }
    assert_server_still_works(&mut server);
}

#[test]
fn truncated_valid_frames_never_panic_or_desync_the_server() {
    let mut server = McpServer::new();
    for frame in representative_valid_frames() {
        // Every character-boundary prefix, including the empty string and
        // the whole (untruncated) frame itself.
        for end in 0..=frame.chars().count() {
            let prefix: String = frame.chars().take(end).collect();
            drive_hostile_line(&mut server, &prefix);
        }
    }
    assert_server_still_works(&mut server);
}

#[test]
fn structurally_valid_but_wrong_shaped_json_never_panics_or_desyncs_the_server() {
    let mut server = McpServer::new();
    for line in wrong_shape_valid_json() {
        drive_hostile_line(&mut server, &line);
    }
    assert_server_still_works(&mut server);
}

/// One server surviving hundreds of garbage lines, truncations, and
/// wrong-shaped frames interleaved with occasional GENUINE valid
/// requests — the closest analog to a real hostile session, where an
/// agent's malformed tool call and a well-formed one land back to back
/// on the same long-lived connection.
#[test]
fn interleaved_hostile_and_genuine_lines_never_desync_the_server() {
    let mut server = McpServer::new();
    let mut rng = Xorshift64::new(0xDEAD_BEEF_F00D_1234);
    let genuine = [
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"hew_describe_scene","arguments":{}}}"#.to_string(),
    ];
    for i in 0..400 {
        if i % 17 == 0 {
            drive_hostile_line(&mut server, &genuine[(i / 17) % genuine.len()]);
        } else if i % 5 == 0 {
            let wrong = wrong_shape_valid_json();
            drive_hostile_line(&mut server, &wrong[i % wrong.len()]);
        } else {
            let line = random_garbage_line(&mut rng, 60);
            drive_hostile_line(&mut server, &line);
        }
    }
    assert_server_still_works(&mut server);
}
