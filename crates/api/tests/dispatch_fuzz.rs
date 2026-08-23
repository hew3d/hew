//! Dispatch-level fuzz harness — the API's public surface driven with
//! hostile input instead of well-formed transcripts, mirroring
//! `crates/kernel/tests/document_fuzz.rs`'s posture (random shapes,
//! always-on invariant checks, no panic tolerated — proptest surfaces a
//! panic as a test failure on its own, so no `catch_unwind` is needed
//! anywhere here) one layer up: `conformance.rs` pins the CONTRACT with
//! literal examples and a small SAFE proptest vocabulary (docs/agents/HEW_API.md
//! §14); this file drives the same `Connection::dispatch` with
//! genuinely adversarial envelopes and asks only the invariants the
//! protocol itself promises (§4.4's error-code inventory, §6.1's
//! one-envelope-one-undo, §6's atomicity) — never that any particular
//! hostile input succeeds or fails a specific way.
//!
//! Two suites:
//!
//! 1. `hostile_envelopes_never_panic_or_corrupt` — one arbitrary `Request`
//!    (real/near-miss/garbage method, an arbitrary bounded JSON tree as
//!    `params`, an arbitrary/absent `id`) dispatched against a small
//!    hello'd+attached document. Every reply must be well-formed (§4.4);
//!    every error code must be drawn from the protocol's inventory; a
//!    refusal's `data` must carry the canonical five fields; and the
//!    document must be untouched by anything that isn't a genuine
//!    one-undo-entry mutating success.
//! 2. `transaction_envelopes_are_atomic_under_hostile_command_sequences` —
//!    random 2-6 command `hew.doc.transact` envelopes drawn from a mixed
//!    vocabulary of real methods, wired with random labels and random
//!    `$ref`/`$face` targets (valid chains, dangling labels, forward
//!    references, right-label-wrong-pointer, and plain literal garbage)
//!    plus randomly placed `hew.context.enter`/`exit`. Every rejection or
//!    refusal — static or at execution — must leave the document
//!    byte-identical (§6's atomicity); every success adds at most one
//!    undo entry, which undoes cleanly when it added one.
//!
//! Suite 3 (MCP-frame fuzz) lives in
//! `crates/hew-cli/tests/mcp_fuzz.rs`, against `hew_cli::mcp::McpServer`
//! directly — a different crate and a different-shaped surface (raw
//! newline-delimited JSON-RPC text), not this dispatcher.
//!
//! Honesty note (mirrors `conformance.rs`'s carve-out style): a mutating
//! success's undo is asserted to SUCCEED and to restore the prior undo
//! depth, always — that is an unconditional kernel/API obligation. Byte-
//! for-byte identity after that undo is NOT asserted as an unconditional
//! property here — not because of any presently-known gap (the one this
//! note used to cite, `follow_me`'s `restore_edges` slot reversal, is
//! fixed; see `crates/kernel/tests/region_consume_undo_reversal_specs.rs`
//! and `conformance.rs`'s SAFE vocabulary, which now covers `follow_me`
//! byte-identically too) — but because this harness's whole point is
//! adversarial, unvetted command sequences: asserting full byte-identity
//! here would mean any future kernel-side non-byte-identical-but-
//! otherwise-correct undo (a new op, a new edge case) fails a HOSTILE-
//! fuzz test rather than a conformance one, muddying which suite owns
//! that contract. The assertion below claims only what this file's
//! adversarial framing is for: depth restored, undo did not error or
//! corrupt anything.

use api::{Connection, DispatchOutcome, NoHost, Profile, Registry, Request, RequestId, Response};
use proptest::prelude::*;
use serde_json::{Value, json};

// --------------------------------------------------------------- plumbing
//
// Duplicated per test file by this crate's own established convention
// (see conformance.rs's own header note) rather than factored into a
// shared module.

fn req(id: Option<RequestId>, method: String, params: Option<Value>) -> Request {
    Request {
        jsonrpc: "2.0".to_string(),
        id,
        method,
        params,
    }
}

fn hello_attach(conn: &mut Connection, doc: &mut kernel::Document) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        doc,
        &mut NoHost,
        req(
            Some(RequestId::Number(0)),
            "hew.meta.hello".to_string(),
            Some(json!({ "protocol": 1 })),
        ),
    ) else {
        panic!("hello replies")
    };
    assert!(r.error.is_none(), "hello failed: {:?}", r.error);
    let DispatchOutcome::Reply(r) = conn.dispatch(
        doc,
        &mut NoHost,
        req(
            Some(RequestId::Number(1)),
            "hew.doc.attach".to_string(),
            Some(json!({})),
        ),
    ) else {
        panic!("attach replies")
    };
    assert!(r.error.is_none(), "attach failed: {:?}", r.error);
}

fn ground_rect(corner_a: [f64; 3], corner_b: [f64; 3]) -> Value {
    json!({ "plane": { "ground": true }, "corner_a": corner_a, "corner_b": corner_b })
}

/// Seeds a small, non-empty document (one box) the same way
/// `conformance.rs`'s `build_box` does — through the dispatcher itself,
/// as a two-command transaction — so both suites fuzz against a document
/// with real (if unpredictable-to-the-fuzzer) content, not an empty one.
fn seed_document(conn: &mut Connection, doc: &mut kernel::Document) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        doc,
        &mut NoHost,
        req(
            Some(RequestId::Number(2)),
            "hew.doc.transact".to_string(),
            Some(json!({
                "commands": [
                    { "method": "hew.sketch.draw_rect", "as": "seed_profile",
                      "params": ground_rect([0.0, 0.0, 0.0], [1.0, 1.0, 0.0]) },
                    { "method": "hew.solid.extrude", "as": "seed_box", "params": {
                        "region": { "$ref": "seed_profile#/region_id" }, "distance": 1.0
                    }}
                ]
            })),
        ),
    ) else {
        panic!("seed transaction replies")
    };
    assert!(
        r.error.is_none(),
        "seeding the fuzz document failed: {:?}",
        r.error
    );
}

/// Every error code the protocol may answer with (docs/agents/HEW_API.md §4.4).
const KNOWN_ERROR_CODES: [i64; 8] = [
    -32700, -32601, -32602, -32000, -32001, -32002, -32003, -32004,
];

/// A reply is well-formed (§4.4) regardless of what hostile input
/// produced it: exactly one of `result`/`error`; any `error.code` is
/// drawn from the protocol's own inventory; and a refusal (`-32000`)
/// always carries the canonical five-field `data` shape.
fn assert_well_formed_reply(response: &Response) -> Result<(), TestCaseError> {
    let v = serde_json::to_value(response)
        .map_err(|e| TestCaseError::fail(format!("response fails to serialize: {e}")))?;
    let has_result = v.get("result").is_some();
    let has_error = v.get("error").is_some();
    if has_result == has_error {
        return Err(TestCaseError::fail(format!(
            "reply must carry exactly one of result/error, got: {v}"
        )));
    }
    if let Some(error) = &response.error {
        if !KNOWN_ERROR_CODES.contains(&error.code) {
            return Err(TestCaseError::fail(format!(
                "error code {} is outside the §4.4 inventory",
                error.code
            )));
        }
        if error.code == api::codes::REFUSED {
            let data = error
                .data
                .as_ref()
                .ok_or_else(|| TestCaseError::fail("a refusal (-32000) must carry error.data"))?;
            for key in [
                "refusal",
                "failed_index",
                "failed_method",
                "detail",
                "explanation",
            ] {
                if data.get(key).is_none() {
                    return Err(TestCaseError::fail(format!(
                        "refusal data lacks the canonical '{key}' field: {data}"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Dispatches `hew.history.undo` and returns whether it succeeded —
/// shared by both suites' "a mutating success undoes cleanly" check.
fn undo_succeeds(conn: &mut Connection, doc: &mut kernel::Document) -> Result<bool, TestCaseError> {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        doc,
        &mut NoHost,
        req(
            Some(RequestId::Number(999_000)),
            "hew.history.undo".to_string(),
            Some(json!({})),
        ),
    ) else {
        return Err(TestCaseError::fail(
            "hew.history.undo is a request, never a notification",
        ));
    };
    Ok(r.error.is_none())
}

// ============================================================ suite 1 ===
// hostile-envelope fuzz: one arbitrary Request, never panic, never corrupt.

/// The full protocol-1 method-name universe, for the "real name" and
/// "near-miss" method strategies below.
fn registry_method_names() -> Vec<String> {
    Registry::protocol_1()
        .commands()
        .map(|c| c.name.to_string())
        .collect()
}

/// A registry name, perturbed into something close-but-wrong: still
/// method-name-shaped, but not itself a real command — exercises
/// `-32601` (and, for the namespace-truncation case, a real namespace
/// prefix that is not itself a full method) right at the boundary of
/// what `Registry::get` accepts.
fn near_miss_method(name: &str, kind: u8) -> String {
    match kind % 6 {
        0 => name[..name.len().saturating_sub(1)].to_string(), // drop last char
        1 => format!("{name}x"),                               // trailing garbage
        2 => name.to_uppercase(),                              // wrong case
        3 => name.replace('.', ".."),                          // doubled separator
        4 => name
            .rsplit_once('.')
            .map(|(ns, _)| ns.to_string())
            .unwrap_or_else(|| name.to_string()), // bare namespace
        _ => format!("{name}.extra"),                          // extra segment
    }
}

fn arb_method(names: Vec<String>) -> impl Strategy<Value = String> {
    let names_for_real = names.clone();
    let names_for_near = names;
    prop_oneof![
        5 => prop::sample::select(names_for_real),
        3 => (prop::sample::select(names_for_near), any::<u8>())
            .prop_map(|(name, kind)| near_miss_method(&name, kind)),
        1 => "[a-zA-Z0-9_. ]{0,40}",
        1 => proptest::collection::vec(any::<char>(), 0..24)
            .prop_map(|cs| cs.into_iter().collect::<String>()),
    ]
}

fn arb_number() -> impl Strategy<Value = Value> {
    prop_oneof![
        3 => (-1000i64..1000).prop_map(|n| json!(n)),
        2 => (-1.0e6f64..1.0e6f64).prop_map(|f| json!(f)),
        1 => Just(json!(f64::MAX)),
        1 => Just(json!(f64::MIN)),
        1 => Just(json!(1.0e300)),
        1 => Just(json!(-1.0e300)),
        1 => Just(json!(0)),
        1 => Just(json!(-0.0)),
        1 => Just(json!(u64::MAX)),
        1 => Just(json!(i64::MIN)),
    ]
}

/// A key that sometimes lands on a real command's field name — so an
/// occasional generated object is actually well-shaped enough to reach a
/// handler's success path — and sometimes is plain garbage.
fn arb_key() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(vec![
            "plane", "ground", "origin", "normal", "center", "radius", "distance",
            "corner_a", "corner_b", "region", "face", "id", "ids", "name", "color",
            "path", "translation", "axis", "angle", "pivot", "object", "at",
            "point", "of", "commands", "method", "as", "params", "label",
            "$ref", "$face",
        ])
        .prop_map(str::to_string),
        1 => "[a-z_]{0,10}",
    ]
}

/// A string leaf: mostly small ASCII, occasionally a huge (~1KB) string,
/// occasionally an id-shaped-but-nonexistent string, occasionally raw
/// unicode/control characters.
fn arb_string_leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        4 => "[a-zA-Z0-9_ ]{0,16}".prop_map(|s| json!(s)),
        1 => Just(json!("x".repeat(1024))),
        1 => prop::sample::select(vec!["obj_deadbeef", "rgn_1_2", "", "cmp_0"])
            .prop_map(|s| json!(s)),
        1 => proptest::collection::vec(any::<char>(), 0..20)
            .prop_map(|cs| json!(cs.into_iter().collect::<String>())),
    ]
}

fn arb_leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        2 => Just(Value::Null),
        2 => proptest::bool::ANY.prop_map(Value::Bool),
        3 => arb_number(),
        3 => arb_string_leaf(),
    ]
}

/// A bounded, arbitrary JSON tree: depth capped at 4, at most ~6 items
/// per array/object — big enough to reach nested params fields (a
/// `plane` object inside a draw command, say) without proptest cases
/// blowing up. Occasionally produces a `$ref`/`$face` reference object
/// with a random label — always caught statically when reached through a
/// single-command auto-wrapped envelope (§6.2: no earlier label can ever
/// exist there), so this exercises `transact.rs`'s static rejection path
/// even outside suite 2's explicit transaction fuzzing.
fn arb_json() -> impl Strategy<Value = Value> {
    arb_leaf().prop_recursive(4, 64, 6, |inner| {
        prop_oneof![
            3 => proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            4 => proptest::collection::vec((arb_key(), inner.clone()), 0..6).prop_map(|kvs| {
                let mut map = serde_json::Map::new();
                for (k, v) in kvs {
                    map.insert(k, v);
                }
                Value::Object(map)
            }),
            1 => ("[a-zA-Z_]{1,8}", "[a-zA-Z_/]{0,12}")
                .prop_map(|(label, ptr)| json!({ "$ref": format!("{label}#{ptr}") })),
            1 => ("[a-zA-Z_]{1,8}", "[a-zA-Z_.]{0,8}")
                .prop_map(|(label, key)| json!({ "$face": format!("{label}#{key}") })),
        ]
    })
}

fn arb_params() -> impl Strategy<Value = Option<Value>> {
    prop_oneof![
        1 => Just(None),
        8 => arb_json().prop_map(Some),
    ]
}

fn arb_request_id() -> impl Strategy<Value = Option<RequestId>> {
    prop_oneof![
        1 => Just(None),
        3 => any::<i64>().prop_map(|n| Some(RequestId::Number(n))),
        3 => "[a-zA-Z0-9_-]{0,12}".prop_map(|s| Some(RequestId::Text(s))),
    ]
}

fn arb_jsonrpc() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => Just("2.0".to_string()),
        1 => Just("1.0".to_string()),
        1 => "[a-zA-Z0-9.]{0,5}",
    ]
}

fn arb_hostile_request() -> impl Strategy<Value = Request> {
    (
        arb_jsonrpc(),
        arb_request_id(),
        arb_method(registry_method_names()),
        arb_params(),
    )
        .prop_map(|(jsonrpc, id, method, params)| Request {
            jsonrpc,
            id,
            method,
            params,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Never panics (proptest turns a panic into a failing case on its
    /// own — no `catch_unwind` needed); every reply is well-formed
    /// (§4.4); and the document is untouched by anything that is not a
    /// genuine one-undo-entry mutating success, which itself must undo
    /// cleanly.
    #[test]
    fn hostile_envelopes_never_panic_or_corrupt(request in arb_hostile_request()) {
        let mut conn = Connection::new(Profile::Core, "fuzz-dispatch");
        let mut doc = kernel::Document::new();
        hello_attach(&mut conn, &mut doc);
        seed_document(&mut conn, &mut doc);

        let bytes_before = doc.save();
        let depth_before = doc.undo_depth();

        let method_name = request.method.clone();
        let outcome = conn.dispatch(&mut doc, &mut NoHost, request);

        match outcome {
            DispatchOutcome::Dropped => {
                // A client-originated notification is dropped UNEXECUTED
                // (§4.1) — it must never reach the document at all.
                prop_assert_eq!(
                    doc.save(), bytes_before,
                    "a dropped notification must never mutate the document"
                );
                prop_assert_eq!(doc.undo_depth(), depth_before);
            }
            DispatchOutcome::Reply(response) => {
                assert_well_formed_reply(&response)?;
                let depth_after = doc.undo_depth();

                if response.error.is_some() {
                    prop_assert_eq!(
                        doc.save(), bytes_before,
                        "a refused/errored envelope must leave the document untouched"
                    );
                    prop_assert_eq!(depth_after, depth_before);
                } else if depth_after + 1 == depth_before {
                    // `hew.history.undo` is itself a reachable method name
                    // in this fuzz's vocabulary (it's Solitary, so it can
                    // be the plain top-level method, or the sole command
                    // of a hostilely-shaped `hew.doc.transact`) — a
                    // successful undo legitimately POPS the one entry
                    // `seed_document` recorded. That is history navigation,
                    // not corruption: the document is expected to change
                    // (back to its pre-seed state), so no byte-identity
                    // claim applies here.
                } else if depth_after == depth_before {
                    // A zero-undo-entry success is read-only or solitary —
                    // no byte change — EXCEPT the three §6.4 registry-state
                    // commands, which mutate the serialized registries
                    // while deliberately recording no undo entry, and the
                    // Scenes family (§7.1): Scenes are persisted view
                    // state outside undo history (docs/design/scenes.md
                    // §3.1), so add/update/rename/describe/remove/reorder
                    // change the saved bytes with no undo entry, and apply
                    // writes the persisted tag/hidden/section view state.
                    let registry_state = matches!(
                        method_name.as_str(),
                        "hew.material.create" | "hew.tag.create" | "hew.tag.set_visible"
                    ) || (method_name.starts_with("hew.scenes.") && method_name != "hew.scenes.list");
                    if !registry_state {
                        prop_assert_eq!(
                            doc.save(), bytes_before,
                            "a success that added no undo entry must not change the document bytes"
                        );
                    }
                } else {
                    prop_assert_eq!(
                        depth_after, depth_before + 1,
                        "a mutating success adds exactly one undo entry (§6.1)"
                    );
                    let ok = undo_succeeds(&mut conn, &mut doc)?;
                    prop_assert!(ok, "hew.history.undo must restore a mutating success cleanly");
                    prop_assert_eq!(
                        doc.undo_depth(), depth_before,
                        "undo must restore the prior undo depth"
                    );
                }
            }
        }
    }
}

// ============================================================ suite 2 ===
// transaction-envelope fuzz: random real-method command sequences, random
// labels, random $ref/$face targets (valid/dangling/forward/wrong-pointer/
// literal), random context enter/exit placement.

#[derive(Debug, Clone, Copy)]
enum RefMode {
    Valid,
    Dangling,
    Forward,
    WrongPointer,
    Literal,
}

fn arb_ref_mode() -> impl Strategy<Value = RefMode> {
    prop_oneof![
        3 => Just(RefMode::Valid),
        2 => Just(RefMode::Dangling),
        1 => Just(RefMode::Forward),
        1 => Just(RefMode::WrongPointer),
        1 => Just(RefMode::Literal),
    ]
}

/// A small, mostly-colliding label pool: heavy on `None` and on a
/// handful of repeated short names, so duplicate labels (a static
/// rejection — §6.2) come up often on their own.
fn arb_label() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        2 => Just(None),
        1 => Just(Some("a".to_string())),
        1 => Just(Some("b".to_string())),
        1 => Just(Some("c".to_string())),
    ]
}

/// One command in a fuzzed transaction envelope, abstractly — resolved
/// to concrete JSON by `build_commands` once the whole sequence (and
/// thus every label's position) is known.
#[derive(Debug, Clone)]
enum StepSpec {
    DrawRect {
        label: Option<String>,
        ox: f64,
        oy: f64,
    },
    DrawCircle {
        label: Option<String>,
        ox: f64,
        oy: f64,
        radius: f64,
    },
    Extrude {
        label: Option<String>,
        region_ref: RefMode,
        distance: f64,
    },
    PushPull {
        label: Option<String>,
        face_ref: RefMode,
        distance: f64,
    },
    Rotate {
        label: Option<String>,
        ids_ref: RefMode,
        angle: f64,
    },
    Move {
        label: Option<String>,
        ids_ref: RefMode,
        translation: (f64, f64, f64),
    },
    MaterialCreate {
        label: Option<String>,
        name: String,
    },
    TagCreate {
        label: Option<String>,
        path: String,
    },
    ContextEnter {
        label: Option<String>,
        id_ref: RefMode,
    },
    ContextExit {
        label: Option<String>,
    },
    ReadOnlyQuery {
        label: Option<String>,
        id_ref: RefMode,
    },
    UnknownMethod {
        label: Option<String>,
        name: String,
    },
}

fn label_of(step: &StepSpec) -> Option<&str> {
    match step {
        StepSpec::DrawRect { label, .. }
        | StepSpec::DrawCircle { label, .. }
        | StepSpec::Extrude { label, .. }
        | StepSpec::PushPull { label, .. }
        | StepSpec::Rotate { label, .. }
        | StepSpec::Move { label, .. }
        | StepSpec::MaterialCreate { label, .. }
        | StepSpec::TagCreate { label, .. }
        | StepSpec::ContextEnter { label, .. }
        | StepSpec::ContextExit { label }
        | StepSpec::ReadOnlyQuery { label, .. }
        | StepSpec::UnknownMethod { label, .. } => label.as_deref(),
    }
}

fn arb_step() -> impl Strategy<Value = StepSpec> {
    prop_oneof![
        3 => (arb_label(), 0.0..8.0f64, 0.0..8.0f64)
            .prop_map(|(label, ox, oy)| StepSpec::DrawRect { label, ox, oy }),
        2 => (arb_label(), 0.0..8.0f64, 0.0..8.0f64, 0.1..1.0f64)
            .prop_map(|(label, ox, oy, radius)| StepSpec::DrawCircle { label, ox, oy, radius }),
        3 => (arb_label(), arb_ref_mode(), -2.0..2.0f64)
            .prop_map(|(label, region_ref, distance)| StepSpec::Extrude { label, region_ref, distance }),
        2 => (arb_label(), arb_ref_mode(), -2.0..2.0f64)
            .prop_map(|(label, face_ref, distance)| StepSpec::PushPull { label, face_ref, distance }),
        2 => (arb_label(), arb_ref_mode(), -3.0..3.0f64)
            .prop_map(|(label, ids_ref, angle)| StepSpec::Rotate { label, ids_ref, angle }),
        2 => (arb_label(), arb_ref_mode(), (-3.0..3.0f64, -3.0..3.0f64, -3.0..3.0f64))
            .prop_map(|(label, ids_ref, translation)| StepSpec::Move { label, ids_ref, translation }),
        1 => (arb_label(), "[A-Za-z]{1,8}")
            .prop_map(|(label, name)| StepSpec::MaterialCreate { label, name }),
        1 => (arb_label(), "[A-Za-z]{1,8}")
            .prop_map(|(label, path)| StepSpec::TagCreate { label, path }),
        1 => (arb_label(), arb_ref_mode())
            .prop_map(|(label, id_ref)| StepSpec::ContextEnter { label, id_ref }),
        1 => arb_label().prop_map(|label| StepSpec::ContextExit { label }),
        1 => (arb_label(), arb_ref_mode())
            .prop_map(|(label, id_ref)| StepSpec::ReadOnlyQuery { label, id_ref }),
        1 => (arb_label(), "[a-z_.]{0,20}")
            .prop_map(|(label, name)| StepSpec::UnknownMethod { label, name }),
    ]
}

fn arb_steps() -> impl Strategy<Value = Vec<StepSpec>> {
    proptest::collection::vec(arb_step(), 2..6)
}

/// Resolves a `$ref`-shaped field: `Valid` points at the most recent
/// compatible earlier label; `Forward` at the nearest LATER one (a
/// static rejection per §6.2, unless none exists, in which case it falls
/// back to a dangling label — still a static rejection, just a
/// different named one); `WrongPointer` names a real earlier label but a
/// pointer it never produced (a RUNTIME `ref_resolution_failed`, §6.2);
/// `Dangling` names no label that ever exists; `Literal` isn't a `$ref`
/// at all, just a plain nonexistent-id string (an execution-time
/// `unknown_entity` refusal, never a static one).
fn resolve_ref(mode: RefMode, earlier: &[&str], later: &[&str], pointer: &str) -> Value {
    match mode {
        RefMode::Valid => match earlier.last() {
            Some(label) => json!({ "$ref": format!("{label}#{pointer}") }),
            None => json!("zzz_never_defined_id"),
        },
        RefMode::Dangling => json!({ "$ref": format!("zzz_ghost_label#{pointer}") }),
        RefMode::Forward => match later.first() {
            Some(label) => json!({ "$ref": format!("{label}#{pointer}") }),
            None => json!({ "$ref": format!("zzz_ghost_label#{pointer}") }),
        },
        RefMode::WrongPointer => match earlier.last() {
            Some(label) => json!({ "$ref": format!("{label}#/definitely_not_a_real_field") }),
            None => json!("zzz_never_defined_id"),
        },
        RefMode::Literal => json!("zzz_literal_garbage_id"),
    }
}

/// The `$face`-shaped analog of [`resolve_ref`] — `Literal` falls back
/// to a plain point-locator object (§5.2's `{object, at}` shape) rather
/// than a bare string, since a face param is never just an id.
fn resolve_face_ref(mode: RefMode, earlier: &[&str], later: &[&str], key: &str) -> Value {
    match mode {
        RefMode::Valid => match earlier.last() {
            Some(label) => json!({ "$face": format!("{label}#{key}") }),
            None => json!({ "object": "zzz_never_defined_id", "at": [0.0, 0.0, 0.0] }),
        },
        RefMode::Dangling => json!({ "$face": format!("zzz_ghost_label#{key}") }),
        RefMode::Forward => match later.first() {
            Some(label) => json!({ "$face": format!("{label}#{key}") }),
            None => json!({ "$face": format!("zzz_ghost_label#{key}") }),
        },
        RefMode::WrongPointer => match earlier.last() {
            Some(label) => json!({ "$face": format!("{label}#nonexistent_key") }),
            None => json!({ "$face": "zzz_ghost_label#nonexistent_key" }),
        },
        RefMode::Literal => json!({ "object": "zzz_literal_garbage_id", "at": [0.0, 0.0, 0.0] }),
    }
}

/// Builds the concrete `commands` JSON array for a fuzzed step sequence.
/// `region_positions`/`object_positions` are computed once over the
/// WHOLE sequence so `Forward` can find a genuinely later label.
fn build_commands(steps: &[StepSpec]) -> Value {
    let region_positions: Vec<(usize, &str)> = steps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            StepSpec::DrawRect { .. } | StepSpec::DrawCircle { .. } => label_of(s).map(|l| (i, l)),
            _ => None,
        })
        .collect();
    let object_positions: Vec<(usize, &str)> = steps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            StepSpec::Extrude { .. } => label_of(s).map(|l| (i, l)),
            _ => None,
        })
        .collect();

    let mut commands = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let earlier_regions: Vec<&str> = region_positions
            .iter()
            .filter(|(j, _)| *j < i)
            .map(|(_, l)| *l)
            .collect();
        let later_regions: Vec<&str> = region_positions
            .iter()
            .filter(|(j, _)| *j > i)
            .map(|(_, l)| *l)
            .collect();
        let earlier_objects: Vec<&str> = object_positions
            .iter()
            .filter(|(j, _)| *j < i)
            .map(|(_, l)| *l)
            .collect();
        let later_objects: Vec<&str> = object_positions
            .iter()
            .filter(|(j, _)| *j > i)
            .map(|(_, l)| *l)
            .collect();

        let (method, as_label, params) = match step {
            StepSpec::DrawRect { label, ox, oy } => (
                "hew.sketch.draw_rect",
                label.clone(),
                ground_rect([*ox, *oy, 0.0], [*ox + 0.5, *oy + 0.5, 0.0]),
            ),
            StepSpec::DrawCircle {
                label,
                ox,
                oy,
                radius,
            } => (
                "hew.sketch.draw_circle",
                label.clone(),
                json!({ "plane": { "ground": true }, "center": [*ox, *oy, 0.0], "radius": radius }),
            ),
            StepSpec::Extrude {
                label,
                region_ref,
                distance,
            } => (
                "hew.solid.extrude",
                label.clone(),
                json!({
                    "region": resolve_ref(*region_ref, &earlier_regions, &later_regions, "/region_id"),
                    "distance": distance,
                }),
            ),
            StepSpec::PushPull {
                label,
                face_ref,
                distance,
            } => (
                "hew.solid.push_pull",
                label.clone(),
                json!({
                    "face": resolve_face_ref(*face_ref, &earlier_objects, &later_objects, "top"),
                    "distance": distance,
                }),
            ),
            StepSpec::Rotate {
                label,
                ids_ref,
                angle,
            } => (
                "hew.entity.rotate",
                label.clone(),
                json!({
                    "ids": [resolve_ref(*ids_ref, &earlier_objects, &later_objects, "/object_id")],
                    "pivot": [0.0, 0.0, 0.0],
                    "axis": [0.0, 0.0, 1.0],
                    "angle": angle,
                }),
            ),
            StepSpec::Move {
                label,
                ids_ref,
                translation,
            } => (
                "hew.entity.move",
                label.clone(),
                json!({
                    "ids": [resolve_ref(*ids_ref, &earlier_objects, &later_objects, "/object_id")],
                    "translation": [translation.0, translation.1, translation.2],
                }),
            ),
            StepSpec::MaterialCreate { label, name } => (
                "hew.material.create",
                label.clone(),
                json!({ "name": name, "color": [120, 60, 200] }),
            ),
            StepSpec::TagCreate { label, path } => {
                ("hew.tag.create", label.clone(), json!({ "path": [path] }))
            }
            StepSpec::ContextEnter { label, id_ref } => (
                "hew.context.enter",
                label.clone(),
                json!({ "id": resolve_ref(*id_ref, &earlier_objects, &later_objects, "/object_id") }),
            ),
            StepSpec::ContextExit { label } => ("hew.context.exit", label.clone(), json!({})),
            StepSpec::ReadOnlyQuery { label, id_ref } => (
                "hew.query.entity",
                label.clone(),
                json!({ "id": resolve_ref(*id_ref, &earlier_objects, &later_objects, "/object_id") }),
            ),
            StepSpec::UnknownMethod { label, name } => (
                "hew.fuzz.not_a_real_method",
                label.clone(),
                json!({ "note": name }),
            ),
        };

        let mut command = serde_json::Map::new();
        command.insert("method".to_string(), json!(method));
        if let Some(l) = as_label {
            command.insert("as".to_string(), json!(l));
        }
        command.insert("params".to_string(), params);
        commands.push(Value::Object(command));
    }
    Value::Array(commands)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// A rejected/refused fuzzed transaction — whether the rejection is
    /// static (unknown method, forward `$ref`, unbalanced context,
    /// duplicate label, a misplaced solitary-class step) or at execution
    /// (a runtime `ref_resolution_failed`, a kernel-typed refusal) —
    /// always leaves the document byte-identical (§6's atomicity). A
    /// successful fuzzed transaction adds AT MOST one undo entry (zero
    /// is legitimate too — an all-registry-state/session batch, §6.4's
    /// carve-out generalized rather than hardcoded to specific method
    /// names); when it adds one, undoing it always succeeds and restores
    /// the prior depth.
    #[test]
    fn transaction_envelopes_are_atomic_under_hostile_command_sequences(steps in arb_steps()) {
        let mut conn = Connection::new(Profile::Core, "fuzz-transact");
        let mut doc = kernel::Document::new();
        hello_attach(&mut conn, &mut doc);
        seed_document(&mut conn, &mut doc);

        let commands = build_commands(&steps);
        let bytes_before = doc.save();
        let depth_before = doc.undo_depth();

        let request = req(
            Some(RequestId::Number(3)),
            "hew.doc.transact".to_string(),
            Some(json!({ "commands": commands })),
        );
        let DispatchOutcome::Reply(response) = conn.dispatch(&mut doc, &mut NoHost, request) else {
            return Err(TestCaseError::fail("hew.doc.transact is a request, never a notification"));
        };
        assert_well_formed_reply(&response)?;

        let depth_after = doc.undo_depth();
        if response.error.is_some() {
            prop_assert_eq!(
                doc.save(), bytes_before,
                "a rejected/refused transaction must leave the document untouched (atomicity)"
            );
            prop_assert_eq!(depth_after, depth_before, "a rejected/refused transaction adds no undo entry");
        } else {
            prop_assert!(
                depth_after == depth_before || depth_after == depth_before + 1,
                "a successful transaction adds at most one undo entry: before={depth_before} after={depth_after}"
            );
            if depth_after == depth_before + 1 {
                let ok = undo_succeeds(&mut conn, &mut doc)?;
                prop_assert!(ok, "a successful transaction's one undo entry must undo cleanly");
                prop_assert_eq!(doc.undo_depth(), depth_before, "undo must restore the prior undo depth");
            }
        }
    }
}
