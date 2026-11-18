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
//! 3. `hostile_plugin_declarations_and_replies_never_panic` — arbitrary
//!    plugin command declarations (random ids, random JSON as schemas) go
//!    through `Registry::register_plugin`, where either outcome is fine,
//!    and every command that does register is called with arbitrary
//!    params and completed with an arbitrary `ForwardReply`; so is a
//!    known-good command. Every response must be well-formed (§4.4) and
//!    answer the request's id, and no forward is left pending.
//!
//! 4. `a_locked_target_only_refuses_locked_node` and
//!    `hostile_envelopes_never_change_a_locked_box` (Lock, OW-1) — every
//!    node-editing command aimed at a locked box answers the
//!    `locked_node` refusal naming it, with the document untouched; and
//!    arbitrary envelopes against a document holding a locked box never
//!    change that box (geometry, name, tags), whatever else they do.
//!
//! The MCP-frame fuzz lives in
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

use api::{
    Connection, DispatchOutcome, Grant, NoHost, Permission, Registry, Request, RequestId, Response,
};
use proptest::prelude::*;
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

// --------------------------------------------------------------- plumbing
//
// Duplicated per test file by this crate's own established convention
// (see conformance.rs's own header note) rather than factored into a
// shared module.

fn req(id: Option<RequestId>, method: String, params: Option<Value>) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id,
        method,
        params,
    }
}

fn hello_attach(conn: &mut Connection, doc: &mut kernel::Document) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        &REGISTRY,
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
        &REGISTRY,
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
        &REGISTRY,
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
        &REGISTRY,
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
/// unicode/control characters — and, when `ids` names real entities of
/// the document under test, often one of those, so an envelope can reach
/// a real node.
fn arb_string_leaf_naming(ids: Vec<String>) -> BoxedStrategy<Value> {
    let junk = prop_oneof![
        4 => "[a-zA-Z0-9_ ]{0,16}".prop_map(|s| json!(s)),
        1 => Just(json!("x".repeat(1024))),
        1 => prop::sample::select(vec!["obj_deadbeef", "rgn_1_2", "", "cmp_0"])
            .prop_map(|s| json!(s)),
        1 => proptest::collection::vec(any::<char>(), 0..20)
            .prop_map(|cs| json!(cs.into_iter().collect::<String>())),
    ];
    if ids.is_empty() {
        junk.boxed()
    } else {
        prop_oneof![
            7 => junk,
            3 => prop::sample::select(ids).prop_map(|s| json!(s)),
        ]
        .boxed()
    }
}

fn arb_leaf_naming(ids: Vec<String>) -> impl Strategy<Value = Value> {
    prop_oneof![
        2 => Just(Value::Null),
        2 => proptest::bool::ANY.prop_map(Value::Bool),
        3 => arb_number(),
        3 => arb_string_leaf_naming(ids),
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
    arb_json_naming(Vec::new())
}

/// [`arb_json`] whose string leaves may name `ids`.
fn arb_json_naming(ids: Vec<String>) -> impl Strategy<Value = Value> {
    arb_leaf_naming(ids).prop_recursive(4, 64, 6, |inner| {
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

fn arb_params_naming(ids: Vec<String>) -> impl Strategy<Value = Option<Value>> {
    prop_oneof![
        1 => Just(None),
        8 => arb_json_naming(ids).prop_map(Some),
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
    arb_hostile_request_naming(Vec::new())
}

/// [`arb_hostile_request`] whose params may name `ids`.
fn arb_hostile_request_naming(ids: Vec<String>) -> impl Strategy<Value = Request> {
    (
        arb_jsonrpc(),
        arb_request_id(),
        arb_method(registry_method_names()),
        arb_params_naming(ids),
    )
        .prop_map(|(jsonrpc, id, method, params)| Request {
            document: None,
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
        hostile_envelope_case(Grant::core(), request)?;
    }

    /// The same invariants under a restricted plugin grant bound to one
    /// document (docs/agents/HEW_API.md §10): the binding, path, and
    /// namespace checks never panic or let a refused envelope through.
    #[test]
    fn hostile_envelopes_never_panic_or_corrupt_under_a_plugin_grant(
        request in arb_hostile_request()
    ) {
        hostile_envelope_case(
            Grant::plugin(
                "com.example.fuzz",
                Some("doc_1"),
                [Permission::ModelRead, Permission::ModelWrite, Permission::AttributesWrite]
                    .into_iter()
                    .collect(),
            ),
            request,
        )?;
    }
}

/// One hostile-envelope case under `grant`.
fn hostile_envelope_case(grant: Grant, request: Request) -> Result<(), TestCaseError> {
    {
        let mut conn = Connection::new(grant, "fuzz-dispatch");
        let mut doc = kernel::Document::new();
        hello_attach(&mut conn, &mut doc);
        seed_document(&mut conn, &mut doc);

        let bytes_before = doc.save();
        let depth_before = doc.undo_depth();

        let method_name = request.method.clone();
        let outcome = conn.dispatch(&REGISTRY, &mut doc, &mut NoHost, request);

        match outcome {
            DispatchOutcome::Forward(f) => {
                prop_assert!(false, "no plugin is registered, yet {} forwarded", f.method);
            }
            DispatchOutcome::Dropped => {
                // A client-originated notification is dropped UNEXECUTED
                // (§4.1) — it must never reach the document at all.
                prop_assert_eq!(
                    doc.save(),
                    bytes_before,
                    "a dropped notification must never mutate the document"
                );
                prop_assert_eq!(doc.undo_depth(), depth_before);
            }
            DispatchOutcome::DryRun(response) => {
                // A rehearsal answers like any reply and never touches the
                // document, success or failure.
                assert_well_formed_reply(&response)?;
                prop_assert_eq!(
                    doc.save(),
                    bytes_before,
                    "a dry run must leave the document untouched"
                );
                prop_assert_eq!(doc.undo_depth(), depth_before);
            }
            DispatchOutcome::Reply(response) => {
                assert_well_formed_reply(&response)?;
                let depth_after = doc.undo_depth();

                if response.error.is_some() {
                    prop_assert_eq!(
                        doc.save(),
                        bytes_before,
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
                    ) || (method_name.starts_with("hew.scenes.")
                        && method_name != "hew.scenes.list");
                    if !registry_state {
                        prop_assert_eq!(
                            doc.save(),
                            bytes_before,
                            "a success that added no undo entry must not change the document bytes"
                        );
                    }
                } else {
                    prop_assert_eq!(
                        depth_after,
                        depth_before + 1,
                        "a mutating success adds exactly one undo entry (§6.1)"
                    );
                    let ok = undo_succeeds(&mut conn, &mut doc)?;
                    prop_assert!(
                        ok,
                        "hew.history.undo must restore a mutating success cleanly"
                    );
                    prop_assert_eq!(
                        doc.undo_depth(),
                        depth_before,
                        "undo must restore the prior undo depth"
                    );
                }
            }
        }
    }
    Ok(())
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
        let mut conn = Connection::new(Grant::core(), "fuzz-transact");
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
        let DispatchOutcome::Reply(response) = conn.dispatch(&REGISTRY, &mut doc, &mut NoHost, request) else {
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

// ============================================================ suite 3 ===
// hostile plugin declarations and forward replies: never panic.

/// A plugin id: mostly valid, sometimes reserved or garbage.
fn arb_plugin_id() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => "[a-z][a-z0-9]{0,6}\\.[a-z][a-z0-9-]{0,6}",
        1 => Just("hew.evil".to_string()),
        1 => "[A-Za-z0-9._ -]{0,12}",
    ]
}

/// A command id: mostly valid (dotted or not), sometimes garbage.
fn arb_command_id() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => "[a-z_]{1,6}(\\.[a-z0-9_]{1,6}){0,2}",
        1 => "[A-Za-z0-9._ -]{0,10}",
    ]
}

/// A schema: a closed object built from supported keywords, or any JSON.
fn arb_schema() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(json!({ "type": "number", "minimum": 0 })),
        Just(json!({ "type": "integer", "maximum": 10 })),
        Just(json!({ "type": "string", "minLength": 1, "maxLength": 4 })),
        Just(json!({ "enum": ["a", 1, null] })),
        Just(json!({ "type": "array", "items": { "type": "boolean" }, "maxItems": 2 })),
        Just(json!({ "oneOf": [{ "type": "string" }, { "type": "integer" }] })),
        Just(json!({ "anyOf": [{ "type": "null" }, { "const": 3 }] })),
        arb_json(),
    ];
    prop_oneof![
        3 => proptest::collection::vec(("[a-z]{1,3}", leaf, proptest::bool::ANY), 0..4).prop_map(|props| {
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();
            for (name, schema, needed) in props {
                if needed {
                    required.push(json!(name.clone()));
                }
                properties.insert(name, schema);
            }
            json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
        }),
        1 => arb_json(),
    ]
}

fn arb_contribution() -> impl Strategy<Value = Value> {
    (
        arb_command_id(),
        proptest::bool::ANY,
        "[ -~]{0,12}",
        proptest::option::of("[ -~]{0,12}"),
        proptest::option::of(prop_oneof![
            Just("model.read".to_string()),
            Just("model.write".to_string()),
            Just("documents.other".to_string()),
            Just("all".to_string()),
            "[a-z.:]{0,12}",
        ]),
        any::<u32>(),
        proptest::bool::ANY,
        arb_schema(),
        arb_schema(),
        proptest::collection::vec("[a-z_]{0,8}", 0..3),
    )
        .prop_map(
            |(
                id,
                mutating,
                summary,
                label,
                permission,
                timeout,
                serial,
                params,
                result,
                refusals,
            )| {
                let mut decl = json!({
                    "id": id,
                    "class": if mutating { "mutating" } else { "read_only" },
                    "summary": summary,
                    "timeout_ms": timeout,
                    "concurrency": if serial { "serial" } else { "parallel" },
                    "params": params,
                    "result": result,
                    "refusals": refusals,
                });
                if let Some(label) = label {
                    decl["label"] = json!(label);
                }
                if let Some(permission) = permission {
                    decl["permission"] = json!(permission);
                }
                decl
            },
        )
}

fn arb_forward_reply() -> impl Strategy<Value = api::ForwardReply> {
    use api::{ForwardFailure, ForwardReply};
    prop_oneof![
        4 => arb_json().prop_map(ForwardReply::Ok),
        2 => ("[ -~]{0,12}", "[ -~]{0,20}", arb_json()).prop_map(|(name, explanation, detail)| {
            ForwardReply::Refusal { name, explanation, detail }
        }),
        1 => Just(ForwardReply::Failed(ForwardFailure::Timeout)),
        1 => Just(ForwardReply::Failed(ForwardFailure::Crashed)),
        1 => Just(ForwardReply::Failed(ForwardFailure::Unavailable)),
        1 => Just(ForwardReply::Failed(ForwardFailure::NoHandler)),
        1 => Just(ForwardReply::Failed(ForwardFailure::Cancelled)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn hostile_plugin_declarations_and_replies_never_panic(
        plugin in arb_plugin_id(),
        decls in proptest::collection::vec(arb_contribution(), 0..4),
        calls in proptest::collection::vec((arb_json(), arb_forward_reply()), 1..4),
        good_reply in arb_forward_reply(),
    ) {
        let mut registry = Registry::protocol_1();
        let good: Vec<plugin_manifest::CommandContribution> = serde_json::from_value(json!([{
            "id": "sum", "class": "read_only", "summary": "Adds.",
            "params": { "type": "object", "properties": { "a": { "type": "number" } }, "required": ["a"], "additionalProperties": false },
            "result": { "type": "object", "properties": { "sum": { "type": "number" } }, "required": ["sum"], "additionalProperties": false }
        }]))
        .expect("the known-good declaration parses");
        registry.register_plugin("org.fuzz.known", &good).expect("the known-good plugin registers");
        let before = registry.commands().count();

        // Hostile JSON may not even be a declaration; when it is, either
        // registration outcome is fine.
        let parsed: Result<Vec<plugin_manifest::CommandContribution>, _> =
            serde_json::from_value(Value::Array(decls));
        let registered = match parsed {
            Ok(decls) => match registry.register_plugin(&plugin, &decls) {
                Ok(()) => decls.len(),
                Err(e) => {
                    prop_assert!(!e.name().is_empty() && !e.to_string().is_empty());
                    prop_assert_eq!(registry.commands().count(), before, "a refused registration inserts nothing");
                    0
                }
            },
            Err(_) => 0,
        };
        prop_assert_eq!(registry.commands().count(), before + registered);

        let mut conn = Connection::new(Grant::core(), "fuzz-plugin");
        let mut doc = kernel::Document::new();
        for method in ["hew.meta.hello", "hew.doc.attach"] {
            let params = if method == "hew.meta.hello" { json!({ "protocol": 1 }) } else { json!({}) };
            conn.dispatch(&registry, &mut doc, &mut NoHost, req(Some(RequestId::Number(0)), method.into(), Some(params)));
        }
        let mut targets: Vec<(String, Value)> = registry
            .plugin_commands(&plugin)
            .map(|c| c.name.clone())
            .zip(calls.iter().map(|(params, _)| params.clone()))
            .collect();
        targets.push(("org.fuzz.known.sum".into(), json!({ "a": 1 })));
        let replies = calls.iter().map(|(_, r)| r.clone()).chain([good_reply]);
        for (i, ((method, params), reply)) in targets.into_iter().zip(replies).enumerate() {
            let id = RequestId::Number(100 + i as i64);
            let outcome = conn.dispatch(&registry, &mut doc, &mut NoHost, req(Some(id.clone()), method.clone(), Some(params)));
            let response = match outcome {
                DispatchOutcome::Reply(r) | DispatchOutcome::DryRun(r) => r,
                DispatchOutcome::Forward(f) => {
                    prop_assert_eq!(&f.method, &method);
                    prop_assert!(f.timeout_ms <= 50_000);
                    conn.complete_forward(&registry, f.token, reply)
                }
                DispatchOutcome::Dropped => {
                    return Err(TestCaseError::fail("a request with an id was dropped"));
                }
            };
            assert_well_formed_reply(&response)?;
            prop_assert_eq!(response.id, Some(id));
        }
        prop_assert_eq!(conn.pending_forward_count(), 0);
    }
}

// ============================================================ suite 4 ===
// Lock (OW-1): a locked target never produces anything but `locked_node`
// or a read.

/// One dispatch on the fixture's connection that must succeed; its
/// command's result.
fn call_ok(
    conn: &mut Connection,
    doc: &mut kernel::Document,
    method: &str,
    params: Value,
) -> Value {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        &REGISTRY,
        doc,
        &mut NoHost,
        req(Some(RequestId::Number(5)), method.to_string(), Some(params)),
    ) else {
        panic!("{method} replies")
    };
    assert!(r.error.is_none(), "{method} failed: {:?}", r.error);
    let result = r.result.expect("a result");
    // A mutating command's answer is wrapped in its transaction's results.
    result
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|r| r.first())
        .cloned()
        .unwrap_or(result)
}

/// The public ids a lock fixture names.
#[derive(Debug, Clone, PartialEq)]
struct LockIds {
    /// The locked box (tagged "Walls", wearing "Paint").
    locked: String,
    /// A free box beside it.
    other: String,
    /// A locked component copy and its component.
    instance: String,
    component: String,
    /// "Paint", which the locked box wears, and "Varnish", which nothing
    /// wears.
    paint: String,
    varnish: String,
}

impl LockIds {
    fn all(&self) -> Vec<String> {
        vec![
            self.locked.clone(),
            self.other.clone(),
            self.instance.clone(),
            self.component.clone(),
            self.paint.clone(),
            self.varnish.clone(),
        ]
    }
}

/// A document over a fresh connection holding: a unit box at the origin,
/// tagged "Walls" and wearing "Paint", locked; a free box beside it; a
/// third box made a component, its one copy locked. The locked box, the
/// copy, and every public id. A fresh document numbers its entities the
/// same way each time, so every fixture has the same ids.
fn locked_box_document() -> (
    Connection,
    kernel::Document,
    kernel::ObjectId,
    kernel::InstanceId,
    LockIds,
) {
    let mut conn = Connection::new(Grant::core(), "fuzz-lock");
    let mut doc = kernel::Document::new();
    hello_attach(&mut conn, &mut doc);
    seed_document(&mut conn, &mut doc);
    for x in [3.0, 6.0] {
        call_ok(
            &mut conn,
            &mut doc,
            "hew.doc.transact",
            json!({
                "commands": [
                    { "method": "hew.sketch.draw_rect", "as": "p",
                      "params": ground_rect([x, 0.0, 0.0], [x + 1.0, 1.0, 0.0]) },
                    { "method": "hew.solid.extrude", "params": {
                        "region": { "$ref": "p#/region_id" }, "distance": 1.0
                    }}
                ]
            }),
        );
    }
    let mut objects = doc.visible_object_ids();
    let min_x = |doc: &kernel::Document, o: kernel::ObjectId| {
        doc.object(o)
            .unwrap()
            .vertices()
            .values()
            .map(|v| v.position.x)
            .fold(f64::MAX, f64::min)
    };
    objects.sort_by(|a, b| min_x(&doc, *a).total_cmp(&min_x(&doc, *b)));
    let (locked, other, third) = (objects[0], objects[1], objects[2]);
    let id_of = |doc: &kernel::Document, e: kernel::EntityRef| {
        api::ids::public_id(&e, doc.sid_of(&e).expect("a stable id"))
    };
    let locked_id = id_of(&doc, kernel::EntityRef::Object(locked));
    let other_id = id_of(&doc, kernel::EntityRef::Object(other));
    let third_id = id_of(&doc, kernel::EntityRef::Object(third));
    let paint = call_ok(
        &mut conn,
        &mut doc,
        "hew.material.create",
        json!({ "name": "Paint", "color": [200, 40, 40] }),
    )["material"]
        .as_str()
        .unwrap()
        .to_string();
    let varnish = call_ok(
        &mut conn,
        &mut doc,
        "hew.material.create",
        json!({ "name": "Varnish", "color": [120, 90, 40] }),
    )["material"]
        .as_str()
        .unwrap()
        .to_string();
    call_ok(
        &mut conn,
        &mut doc,
        "hew.material.set_default",
        json!({ "id": locked_id, "material": paint }),
    );
    call_ok(
        &mut conn,
        &mut doc,
        "hew.tag.create",
        json!({ "path": ["Walls"] }),
    );
    call_ok(
        &mut conn,
        &mut doc,
        "hew.tag.assign",
        json!({ "id": locked_id, "path": ["Walls"] }),
    );
    let made = call_ok(
        &mut conn,
        &mut doc,
        "hew.component.create",
        json!({ "members": [third_id] }),
    );
    let instance_id = made["instance"].as_str().unwrap().to_string();
    let component_id = made["component"].as_str().unwrap().to_string();
    call_ok(
        &mut conn,
        &mut doc,
        "hew.entity.lock",
        json!({ "targets": [locked_id, instance_id] }),
    );
    let instance = doc
        .instance_ids()
        .into_iter()
        .find(|&i| id_of(&doc, kernel::EntityRef::Instance(i)) == instance_id)
        .expect("the copy");
    let ids = LockIds {
        locked: locked_id,
        other: other_id,
        instance: instance_id,
        component: component_id,
        paint,
        varnish,
    };
    (conn, doc, locked, instance, ids)
}

/// The ids every lock fixture has.
fn lock_ids() -> LockIds {
    locked_box_document().4
}

/// What a lock keeps on a node: every vertex it renders where it stands
/// (an instance through its pose), its name and its tags.
fn node_print(
    doc: &kernel::Document,
    node: kernel::NodeId,
) -> (Vec<[i64; 3]>, Option<String>, Vec<Vec<String>>) {
    let quantize = |p: kernel::Point3| {
        [
            (p.x * 1e6).round() as i64,
            (p.y * 1e6).round() as i64,
            (p.z * 1e6).round() as i64,
        ]
    };
    let mut pts: Vec<[i64; 3]> = Vec::new();
    let name = match node {
        kernel::NodeId::Object(o) => {
            if let Some(obj) = doc.object(o) {
                pts.extend(obj.vertices().values().map(|v| quantize(v.position)));
            }
            doc.object_name(o)
        }
        kernel::NodeId::Instance(i) => {
            if let (Some(pose), Some(def)) = (doc.instance_pose(i), doc.instance_def(i)) {
                for o in doc.def_members(def).unwrap_or_default() {
                    if let Some(obj) = doc.object(o) {
                        pts.extend(
                            obj.vertices()
                                .values()
                                .map(|v| quantize(pose.apply_point(v.position))),
                        );
                    }
                }
            }
            doc.instance_name(i)
        }
        kernel::NodeId::Group(g) => doc.group_name(g),
    };
    pts.sort_unstable();
    (pts, name.map(str::to_string), doc.node_tags(node).to_vec())
}

/// The top face of the locked unit box, as a face locator.
fn top_of(id: &str) -> Value {
    json!({ "object": id, "at": [0.5, 0.5, 1.0] })
}

/// The node-editing commands, each aimed at a locked node of the fixture
/// (`n` a free number), with the id the refusal must name. Commands this
/// build's registry does not serve are left out.
fn aimed_commands(ids: &LockIds, n: f64) -> Vec<(&'static str, Value, String)> {
    let n = n.abs() + 0.5;
    let (id, other) = (ids.locked.as_str(), ids.other.as_str());
    let inst = ids.instance.as_str();
    let on_box = |m: &'static str, p: Value| (m, p, ids.locked.clone());
    let on_copy = |m: &'static str, p: Value| (m, p, ids.instance.clone());
    let all = vec![
        on_box(
            "hew.entity.move",
            json!({ "ids": [id], "translation": [n, 0.0, 0.0] }),
        ),
        on_box(
            "hew.entity.rotate",
            json!({ "ids": [id], "pivot": [0.0, 0.0, 0.0], "axis": [0.0, 0.0, 1.0], "angle": n }),
        ),
        on_box(
            "hew.entity.scale",
            json!({ "ids": [id], "anchor": [0.0, 0.0, 0.0], "factors": [n, n, n] }),
        ),
        on_box("hew.entity.delete", json!({ "id": id })),
        on_box("hew.entity.rename", json!({ "id": id, "name": "Renamed" })),
        on_box("hew.entity.flip", json!({ "ids": [id], "axis": "x" })),
        on_box(
            "hew.entity.set_frame",
            json!({ "id": id, "origin": [n, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0] }),
        ),
        on_box("hew.tag.assign", json!({ "id": id, "path": ["Roof"] })),
        on_box(
            "hew.tag.assign",
            json!({ "id": id, "path": ["Walls"], "remove": true }),
        ),
        on_box(
            "hew.tag.rename",
            json!({ "path": ["Walls"], "new_path": ["Fences"] }),
        ),
        on_box("hew.tag.delete", json!({ "path": ["Walls"] })),
        on_box(
            "hew.attr.set",
            json!({ "target": id, "ns": "com.example.fuzz", "key": "k", "value": n }),
        ),
        on_box("hew.group.create", json!({ "members": [id] })),
        on_box("hew.component.create", json!({ "members": [id] })),
        on_box("hew.solid.union", json!({ "a": other, "b": id })),
        on_box("hew.solid.subtract", json!({ "a": id, "b": other })),
        on_box(
            "hew.solid.slice",
            json!({ "object": id, "plane": { "origin": [0.5, 0.5, 0.5], "normal": [1.0, 0.0, 0.0] } }),
        ),
        on_box("hew.solid.intersect", json!({ "a": id, "b": other })),
        on_box(
            "hew.solid.push_pull",
            json!({ "face": top_of(id), "distance": n }),
        ),
        on_box(
            "hew.solid.shell",
            json!({ "object": id, "thickness": 0.05 }),
        ),
        on_box(
            "hew.solid.fillet",
            json!({ "edges": [{ "face": top_of(id) }], "radius": 0.1 }),
        ),
        on_box(
            "hew.solid.chamfer",
            json!({ "edges": [{ "face": top_of(id) }], "distance": 0.1 }),
        ),
        on_box(
            "hew.solid.soften",
            json!({ "object": id, "max_angle": 1.7 }),
        ),
        on_box("hew.solid.mend", json!({ "object": id })),
        on_box(
            "hew.material.set_default",
            json!({ "id": id, "material": null }),
        ),
        on_box(
            "hew.material.paint",
            json!({ "face": top_of(id), "material": ids.varnish }),
        ),
        on_box(
            "hew.material.replace",
            json!({ "from": ids.paint, "to": ids.varnish }),
        ),
        on_copy(
            "hew.entity.move",
            json!({ "ids": [inst], "translation": [n, 0.0, 0.0] }),
        ),
        on_copy("hew.component.explode", json!({ "instance": inst })),
        on_copy(
            "hew.component.flip_instance",
            json!({ "instance": inst, "axis": "x" }),
        ),
        on_copy(
            "hew.component.rebase",
            json!({ "id": ids.component, "origin": [n, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0] }),
        ),
        on_copy(
            "hew.entity.rename",
            json!({ "id": inst, "name": "Renamed" }),
        ),
    ];
    let served = registry_method_names();
    all.into_iter()
        .filter(|(m, _, _)| served.iter().any(|s| s == m))
        .collect()
}

/// What a lock allows on a locked node, each aimed at the fixture's box
/// or copy: a linked copy, a separate copy, drawing on its face (a free
/// drawing that does not cut it), Make Unique on the locked copy.
fn allowed_commands(ids: &LockIds) -> Vec<(&'static str, Value)> {
    vec![
        (
            "hew.entity.move",
            json!({ "ids": [ids.locked], "translation": [0.0, 2.0, 0.0], "copy": { "count": 1 } }),
        ),
        (
            "hew.entity.move",
            json!({ "ids": [ids.locked], "translation": [0.0, 2.0, 0.0],
                    "copy": { "count": 1, "linked": false } }),
        ),
        (
            "hew.sketch.draw_rect",
            json!({ "plane": { "face": top_of(&ids.locked) },
                    "corner_a": [0.25, 0.25, 1.0], "corner_b": [0.75, 0.75, 1.0] }),
        ),
        (
            "hew.component.make_unique",
            json!({ "instance": ids.instance }),
        ),
    ]
}

/// Requests aimed at the fixture's locked nodes and then perturbed: a
/// well-formed node edit with one field dropped or replaced by arbitrary
/// JSON (which may name any fixture id), bare or in a transaction. The
/// near-miss envelopes a hostile client sends, reaching real nodes.
fn arb_aimed_request(ids: LockIds) -> impl Strategy<Value = Request> {
    let count = aimed_commands(&ids, 0.0).len();
    (
        0..count,
        -3.0f64..3.0,
        proptest::option::of((
            any::<usize>(),
            proptest::option::of(arb_json_naming(ids.all())),
        )),
        proptest::bool::ANY,
    )
        .prop_map(move |(which, n, mutation, wrapped)| {
            let (method, mut params, _) = aimed_commands(&ids, n).swap_remove(which);
            if let (Some((key, value)), Some(map)) = (mutation, params.as_object_mut()) {
                let keys: Vec<String> = map.keys().cloned().collect();
                let key = &keys[key % keys.len()];
                match value {
                    Some(v) => map.insert(key.clone(), v),
                    None => map.remove(key),
                };
            }
            if wrapped {
                req(
                    Some(RequestId::Number(10)),
                    "hew.doc.transact".to_string(),
                    Some(json!({ "commands": [ { "method": method, "params": params } ] })),
                )
            } else {
                req(
                    Some(RequestId::Number(10)),
                    method.to_string(),
                    Some(params),
                )
            }
        })
}

/// Every node-editing command aimed at a locked node, bare or inside a
/// transaction, answers `locked_node` naming that node (not through a
/// shared shape), and the document is untouched.
#[test]
fn a_locked_target_only_refuses_locked_node() {
    let ids = lock_ids();
    let commands = aimed_commands(&ids, 1.0);
    assert!(
        commands.len() >= 25,
        "the list is served: {}",
        commands.len()
    );
    for (method, params, cause) in commands {
        for wrapped in [false, true] {
            let (mut conn, mut doc, _, _, _) = locked_box_document();
            let bytes = doc.save();
            let depth = doc.undo_depth();
            let request = if wrapped {
                req(
                    Some(RequestId::Number(10)),
                    "hew.doc.transact".to_string(),
                    Some(json!({ "commands": [ { "method": method, "params": params } ] })),
                )
            } else {
                req(
                    Some(RequestId::Number(10)),
                    method.to_string(),
                    Some(params.clone()),
                )
            };
            let DispatchOutcome::Reply(r) =
                conn.dispatch(&REGISTRY, &mut doc, &mut NoHost, request)
            else {
                panic!("{method} replies");
            };
            assert_well_formed_reply(&r).unwrap();
            let error = r
                .error
                .as_ref()
                .unwrap_or_else(|| panic!("{method} {params} on a locked node succeeded"));
            assert_eq!(error.code, api::codes::REFUSED, "{method}: {error:?}");
            let data = error.data.as_ref().expect("refusal data");
            assert_eq!(data["refusal"], "locked_node", "{method} {params}: {data}");
            assert_eq!(data["detail"]["node"], json!(cause), "{method}: {data}");
            assert_eq!(data["detail"]["via_shared_shape"], json!(false), "{method}");
            assert!(data["explanation"].as_str().is_some_and(|e| !e.is_empty()));
            assert_eq!(doc.save(), bytes, "{method} changed the document");
            assert_eq!(doc.undo_depth(), depth, "{method}");
        }
    }
}

/// What a lock allows succeeds on the locked box and copy, and neither
/// changes nor comes free. (A linked copy of the box moves its lock onto
/// the new shape, so the box stays frozen as that shape's member.)
#[test]
fn what_a_lock_allows_leaves_the_locked_nodes_as_they_were() {
    let ids = lock_ids();
    for (method, params) in allowed_commands(&ids) {
        let (mut conn, mut doc, locked, instance, _) = locked_box_document();
        let nodes = [
            kernel::NodeId::Object(locked),
            kernel::NodeId::Instance(instance),
        ];
        let before: Vec<_> = nodes.iter().map(|&n| node_print(&doc, n)).collect();
        call_ok(&mut conn, &mut doc, method, params.clone());
        for (k, &n) in nodes.iter().enumerate() {
            assert!(
                doc.node_frozen(n).is_some(),
                "{method} {params}: {n:?} stays frozen"
            );
            assert_eq!(node_print(&doc, n), before[k], "{method} {params}: {n:?}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Arbitrary envelopes against a document holding a locked box and a
    /// locked component copy, half of them perturbed node edits aimed at
    /// those nodes and the rest free-form JSON that may name any of the
    /// document's ids: neither locked node ever changes, and a
    /// `locked_node` refusal always names a node and says whether the
    /// lock came through a shared shape.
    #[test]
    fn hostile_envelopes_never_change_a_locked_box(
        request in prop_oneof![
            1 => arb_hostile_request_naming(lock_ids().all()),
            1 => arb_aimed_request(lock_ids()),
        ]
    ) {
        let (mut conn, mut doc, locked, instance, ids) = locked_box_document();
        prop_assert_eq!(&ids, &lock_ids(), "every fixture has the same ids");
        let nodes = [kernel::NodeId::Object(locked), kernel::NodeId::Instance(instance)];
        let before: Vec<_> = nodes.iter().map(|&n| node_print(&doc, n)).collect();
        let outcome = conn.dispatch(&REGISTRY, &mut doc, &mut NoHost, request);
        if let DispatchOutcome::Reply(r) | DispatchOutcome::DryRun(r) = &outcome {
            assert_well_formed_reply(r)?;
            if let Some(data) = r.error.as_ref().and_then(|e| e.data.as_ref())
                && data["refusal"] == "locked_node"
            {
                prop_assert!(data["detail"]["node"].is_string(), "{}", data);
                prop_assert!(data["detail"]["via_shared_shape"].is_boolean(), "{}", data);
            }
        }
        // History navigation may take the lock itself back (undo pops it),
        // after which a node is free: compare only while it is frozen. A
        // linked copy of the box moves its lock onto the new shape (the
        // box stays frozen as its member); Make Unique on the locked copy
        // repoints it onto a new locked shape, the same geometry where it
        // stands.
        for (k, &n) in nodes.iter().enumerate() {
            if doc.node_frozen(n).is_some() {
                prop_assert_eq!(node_print(&doc, n), before[k].clone(), "{:?} changed while frozen", n);
            }
        }
    }
}
