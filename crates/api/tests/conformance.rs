//! The API conformance suite (docs/agents/HEW_API.md §14) — the contract's
//! teeth. Four parts:
//!
//! 1. **Golden transcripts** — literal request/response fixtures under
//!    `tests/transcripts/`, replayed against the dispatcher and compared
//!    STRUCTURALLY (parsed-JSON equality, so a serializer's cosmetic
//!    choices can never break a golden).
//! 2. **Property tests** — the transaction/undo guarantees: a refused
//!    transaction leaves the document byte-identical
//!    (`refused_transactions_leave_the_document_byte_identical`); every
//!    successful model-mutating envelope adds exactly one undo entry and
//!    undoes byte-identically, over a proptest-generated SAFE vocabulary
//!    (`every_mutating_envelope_is_one_undo_entry`), with the one
//!    remaining documented carve-out pinned individually (the test just
//!    above it); and derived points resolve exactly, with ambiguity
//!    refusing rather than guessing
//!    (`derived_points_resolve_exactly_and_ambiguity_refuses`).
//! 3. **Registry completeness** — schemas/summaries/refusal inventories
//!    live with the registry (`registry::tests::every_declaration_is_complete`);
//!    this file extends that over the dispatch surface itself
//!    (`every_implemented_command_has_a_handler_and_vice_versa`). The
//!    generated-artifact byte-identity checks arrive with the generators.
//! 4. **Determinism replay** — `envelope_scripts_replay_byte_identically`.
//!
//! Where this suite and docs/agents/HEW_API.md are ever found to disagree, that
//! is a specification bug fixed in both, in the open — never resolved
//! silently in whichever direction is convenient (§14).

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId, Response, codes};
use proptest::prelude::*;
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

// ------------------------------------------------------- golden transcripts

/// One fixture file: the granted profile (or, when present, the full
/// grant, which wins), the plugins registered before frame 0, and the
/// ordered frames.
#[derive(serde::Deserialize)]
struct Transcript {
    profile: String,
    #[serde(default)]
    grant: Option<serde_json::Value>,
    #[serde(default)]
    plugins: Vec<TranscriptPlugin>,
    frames: Vec<Frame>,
}

#[derive(serde::Deserialize)]
struct TranscriptPlugin {
    id: String,
    commands: Vec<plugin_manifest::CommandContribution>,
}

#[derive(serde::Deserialize)]
struct Frame {
    note: String,
    request: serde_json::Value,
    /// Present when the request must forward: `expect` is compared to the
    /// ForwardRequest's `{plugin, local, params, mutating}` and `reply` is
    /// handed to `complete_forward`.
    #[serde(default)]
    forward: Option<ForwardFrame>,
    /// A JSON pointer: when present, only that part of the response is
    /// compared with `response`.
    #[serde(default)]
    compare: Option<String>,
    /// The outcome kind the dispatcher must answer with: `"reply"` (the
    /// default), or `"dry_run"` for a rehearsal (docs/agents/HEW_API.md
    /// §6.6), whose body alone cannot prove the host was told.
    #[serde(default = "reply_outcome")]
    outcome: String,
    response: serde_json::Value,
}

fn reply_outcome() -> String {
    "reply".into()
}

#[derive(serde::Deserialize)]
struct ForwardFrame {
    expect: serde_json::Value,
    reply: serde_json::Value,
}

/// Structural equality with EXACT f64 bit equality (§14): `serde_json`'s
/// own `Value` equality compares floats with `==`, which calls `0.0` and
/// `-0.0` equal — not good enough for a golden that must pin the
/// serializer's exact numeric output.
fn structurally_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            _ if x.is_f64() != y.is_f64() => false,
            (Some(fx), Some(fy)) if x.is_f64() => fx.to_bits() == fy.to_bits(),
            // Both integral: compare exactly across the i64/u64 split.
            _ => match (x.as_i64(), y.as_i64(), x.as_u64(), y.as_u64()) {
                (Some(ix), Some(iy), _, _) => ix == iy,
                (_, _, Some(ux), Some(uy)) => ux == uy,
                _ => false,
            },
        },
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| structurally_equal(x, y))
        }
        (Value::Object(xs), Value::Object(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .all(|(k, x)| ys.get(k).is_some_and(|y| structurally_equal(x, y)))
        }
        _ => a == b,
    }
}

/// Replays one transcript file against a fresh connection + document,
/// comparing each response structurally.
fn replay(path: &str) {
    let raw = std::fs::read_to_string(path).expect("transcript file");
    let transcript: Transcript = serde_json::from_str(&raw).expect("transcript parses");
    let grant = match (&transcript.grant, transcript.profile.as_str()) {
        (Some(grant), _) => serde_json::from_value(grant.clone()).expect("transcript grant parses"),
        (None, "core") => Grant::core(),
        (None, "app") => Grant::app(),
        (None, other) => panic!("unknown profile {other}"),
    };
    let mut registry = api::Registry::protocol_1();
    for plugin in &transcript.plugins {
        registry
            .register_plugin(&plugin.id, &plugin.commands)
            .unwrap_or_else(|e| panic!("transcript plugin {} registers: {e}", plugin.id));
    }
    let mut conn = Connection::new(grant, "conformance");
    let mut doc = kernel::Document::new();
    let mut host = api::NoHost;
    for (i, frame) in transcript.frames.iter().enumerate() {
        let request: Request =
            serde_json::from_value(frame.request.clone()).expect("request parses");
        let outcome = conn.dispatch(&registry, &mut doc, &mut host, request);
        let kind = match &outcome {
            DispatchOutcome::DryRun(_) => "dry_run",
            _ => "reply",
        };
        if frame.forward.is_none() {
            assert_eq!(
                kind, frame.outcome,
                "frame {i} ({}) answered as the wrong outcome",
                frame.note
            );
        }
        let response = match (outcome, &frame.forward) {
            (DispatchOutcome::Reply(response) | DispatchOutcome::DryRun(response), None) => {
                response
            }
            (DispatchOutcome::Forward(forward), Some(expected)) => {
                let projected = json!({
                    "plugin": forward.plugin,
                    "local": forward.local,
                    "params": forward.params,
                    "mutating": forward.mutating,
                });
                assert!(
                    structurally_equal(&projected, &expected.expect),
                    "frame {i} ({}) forwarded differently\ngot:      {projected}\nexpected: {}",
                    frame.note,
                    expected.expect
                );
                let reply: api::dispatch::ForwardReply =
                    serde_json::from_value(expected.reply.clone()).expect("forward reply parses");
                conn.complete_forward(&registry, forward.token, reply)
            }
            (other, _) => panic!(
                "frame {i} ({}) expected {}, got {other:?}",
                frame.note,
                if frame.forward.is_some() {
                    "a forward"
                } else {
                    "a reply"
                }
            ),
        };
        let whole = serde_json::to_value(&response).expect("response serializes");
        let got = match &frame.compare {
            None => whole,
            Some(pointer) => whole
                .pointer(pointer)
                .cloned()
                .unwrap_or_else(|| panic!("frame {i} ({}): nothing at {pointer}", frame.note)),
        };
        assert!(
            structurally_equal(&got, &frame.response),
            "frame {i} diverged from the golden: {}\ngot:      {got}\nexpected: {}",
            frame.note,
            frame.response
        );
    }
}

#[test]
fn lifecycle_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/lifecycle.json"
    ));
}

/// The §6.1 table-leg shape (`draw_circle` -> extrude via `$ref` -> rename
/// via `$ref`), pinning the full `{results: [...], label}` success shape,
/// followed by a transaction refused at its second command pinning the
/// canonical §4.4 error shape.
#[test]
fn transact_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/transact.json"
    ));
}

/// §6.3's context-balance rejection and §6.2's forward-`$face`-reference
/// rejection: both `-32602`, both caught BEFORE anything runs.
#[test]
fn context_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/context.json"
    ));
}

/// A restricted plugin grant (docs/agents/HEW_API.md §10): the hello echo,
/// the grant gate on plain and transacted commands, the attribute
/// namespace rule, document binding, and path parameters.
#[test]
fn grants_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/grants.json"
    ));
}

/// Plugin-provided commands (docs/agents/HEW_API.md §10.2): validated
/// params, the forward and its completion, a result that breaks its
/// schema, the plugin refusal mapping, the sole-command transaction
/// unwrap, the mixed-transaction refusal, the failure replies, and the
/// capabilities entry.
#[test]
fn forwarding_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/forwarding.json"
    ));
}

/// Dry runs (docs/agents/HEW_API.md §6.6): a rehearsal answers the ids a
/// commit would mint and moves neither the generation nor the undo depth;
/// `coalesce` is `-32602`; a Host-served command refuses
/// `dry_run_host_command`; a stale `expect_generation` refuses first; a
/// rehearsed subtract is read back through `$ref` in the same call.
#[test]
fn dry_run_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/dry_run.json"
    ));
}

/// Pure computations (docs/agents/HEW_API.md §7 "Compute"): measure,
/// boolean, slice, shells, and offset on transient solids, the two
/// reserved rows, and refusals by name.
#[test]
fn compute_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/compute.json"
    ));
}

/// Compare-and-swap and coalescing (docs/agents/HEW_API.md §6.5): a stale
/// `expect_generation` refused `generation_moved` with its five-field
/// error, a current one committing, two commits under one `coalesce` key
/// landing as one entry, and the clock on query results.
#[test]
fn cas_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/cas.json"
    ));
}

/// Event subscription (docs/agents/HEW_API.md §4.5) under a plugin grant
/// holding only `model.read`: an event the grant cannot receive is
/// denied, not refused; an unknown name is `-32602`; unsubscribing
/// answers what is left.
#[test]
fn events_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/events.json"
    ));
}

/// `hew.query.changes_since` (docs/agents/HEW_API.md §4.5): a commit's
/// change set by public id, the undo's swapped lists, the union across
/// both, and `full` for a generation the history does not cover.
#[test]
fn changes_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/changes.json"
    ));
}

/// Selection and view state (docs/agents/HEW_API.md §7) on a host with
/// no selection or viewport: the host-served selection refuses typed, while
/// the kernel-served hide and hidden still answer.
#[test]
fn selection_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/selection.json"
    ));
}

/// The user's editing context from the bus (HEW_API.md §6.3) on a host
/// with no window: the context read reports empty `fallback` and `armed:
/// false`, the three view commands refuse `host_capability_missing`, a
/// target that names nothing is `unknown_entity`, an empty locator and a
/// transaction carrying them are `-32602`.
#[test]
fn user_context_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/user_context.json"
    ));
}

/// The model reads for parts: every placement with its path and composed
/// pose (a mirrored placement included), roots in order, a rotated board's
/// true size, an occurrence locator, topology and mesh counts, and the
/// refusals for an empty root list and a bare definition-member id.
#[test]
fn occurrences_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/occurrences.json"
    ));
}

/// Members transformed and cut through an occurrence path
/// (HEW_API.md §7): a member moved along world +x through a mirrored
/// placement moves +x there and −x in the plain placement, two copies land
/// in the definition and show in every placement, a member turned about a
/// world axis, ids from two scopes refused, a world cutter subtracted
/// through the mirrored path with the result listed under the plain
/// placement too, and a member moved through a quarter-turned placement
/// (a pose that is not its own inverse, so the conjugation order shows).
#[test]
fn member_transforms_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/member_transforms.json"
    ));
}

/// Raycast through nested and mirrored placements: the occurrence path,
/// leaf, and face locator of a hit, `exclude` by id and by path, a mirrored
/// face's outward normal, `all` with `max_distance`, and the missed and
/// unknown-entity refusals.
#[test]
fn raycast_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/raycast.json"
    ));
}

/// Measure by id and by path: a cube's volume, area, and box, a group's
/// sum, a stretched nested placement, the member-needs-a-path refusal, the
/// legacy point pair, and the mixed-forms params error.
#[test]
fn measure_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/measure.json"
    ));
}

/// Import as a definition (docs/agents/HEW_API.md §7, document commands): a `.hew` item
/// becomes one unplaced definition, the same bytes reuse it with no second
/// undo entry, and the refusals (a picture, a host-less STL, a stray
/// `path` member) answer typed.
#[test]
fn import_definition_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/import_definition.json"
    ));
}

/// The model writes for parts: a rebase that compensates both placements
/// (each pose takes the frame, the world box stays put), a refused frame,
/// a flip that mirrors one placement, a 2 × 3 grid copy and the
/// grid-with-translation params error, and a part frame override set,
/// refused on an instance, and cleared.
#[test]
fn writes_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/writes.json"
    ));
}

/// Instance materials: one of two placements painted (its leaf's
/// effective material is the paint, the other's none), then the
/// definition painted (a base beats the placement), and a definition id
/// refusing the instance scope.
#[test]
fn instance_materials_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/instance_materials.json"
    ));
}

/// Face and edge attribute dictionaries (docs/agents/HEW_API.md §8) under
/// a plugin grant at the core ceiling: a face locator target written and
/// read back through `hew.attr.get` and `hew.query.faces`, an edge locator
/// target through `hew.query.topology`, the change set's face and edge
/// scopes, the moved face keeping its dictionary across a push/pull while
/// the stretched wall has none, and the refusals for a sketch edge, a
/// namespace outside the grant, and a target naming both forms.
#[test]
fn face_attrs_transcript_matches_the_golden() {
    replay(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/transcripts/face_attrs.json"
    ));
}

/// Notifications never produce a frame — dropped unexecuted (§4.1), so no
/// transcript can hold them; pinned here instead.
#[test]
fn notification_frames_are_dropped_unexecuted() {
    let mut conn = Connection::new(Grant::core(), "conformance");
    let mut doc = kernel::Document::new();
    let notification: Request = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0", "method": "hew.solid.extrude", "params": {}
    }))
    .unwrap();
    assert!(matches!(
        conn.dispatch(&REGISTRY, &mut doc, &mut api::NoHost, notification),
        DispatchOutcome::Dropped
    ));
}

// -------------------------------------------------------------- properties
//
// §14's transaction/undo properties. The dispatcher is implemented
// (3c9eeb8, 50b04fe), so these run for real now instead of being stated
// as `#[ignore]`d burn-down markers.

// --------------------------------------------------- shared dispatch harness
//
// Duplicated per test file by this crate's own established convention
// (commands_sketch_solid.rs, commands_query.rs, commands_misc.rs,
// commands_entity.rs, commands_doc.rs each carry their own copy) rather
// than factored into a shared module — this file owns only itself.

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn hello(conn: &mut Connection, doc: &mut kernel::Document) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        &REGISTRY,
        doc,
        &mut NoHost,
        req(0, "hew.meta.hello", json!({ "protocol": 1 })),
    ) else {
        panic!("hello replies")
    };
    assert!(r.error.is_none(), "hello failed: {:?}", r.error);
}

fn hello_attach(conn: &mut Connection, doc: &mut kernel::Document) {
    hello(conn, doc);
    let DispatchOutcome::Reply(r) = conn.dispatch(
        &REGISTRY,
        doc,
        &mut NoHost,
        req(1, "hew.doc.attach", json!({})),
    ) else {
        panic!("attach replies")
    };
    assert!(r.error.is_none(), "attach failed: {:?}", r.error);
}

fn call(
    conn: &mut Connection,
    doc: &mut kernel::Document,
    id: i64,
    method: &str,
    params: Value,
) -> Response {
    let DispatchOutcome::Reply(r) =
        conn.dispatch(&REGISTRY, doc, &mut NoHost, req(id, method, params))
    else {
        panic!("{method} replies")
    };
    r
}

fn call_ok(
    conn: &mut Connection,
    doc: &mut kernel::Document,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, id, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    r.result.expect("a successful reply carries a result")
}

/// Dispatches a command expected to refuse, returning the canonical §4.4
/// `error.data` payload.
fn call_err(
    conn: &mut Connection,
    doc: &mut kernel::Document,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, id, method, params);
    let err = r
        .error
        .unwrap_or_else(|| panic!("{method} was expected to refuse"));
    assert_eq!(
        err.code,
        codes::REFUSED,
        "{method}'s failure should be a typed refusal"
    );
    err.data.expect("a refusal carries the canonical §4.4 data")
}

/// A plain (non-`transact`) model-mutating request dispatches as exactly a
/// one-command transaction (§6.1); its result is the wrapper's
/// `results[0]`, not the bare command result.
fn single(result: Value) -> Value {
    result["results"][0].clone()
}

fn ground_rect(corner_a: [f64; 3], corner_b: [f64; 3]) -> Value {
    json!({ "plane": { "ground": true }, "corner_a": corner_a, "corner_b": corner_b })
}

/// Draws a rectangle then extrudes it into a solid, in one transaction —
/// the §6.1 example's exact chaining shape. Returns the new object's
/// public id.
fn build_box(
    conn: &mut Connection,
    doc: &mut kernel::Document,
    id: i64,
    corner_a: [f64; 3],
    corner_b: [f64; 3],
    distance: f64,
) -> String {
    let result = call_ok(
        conn,
        doc,
        id,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile", "params": ground_rect(corner_a, corner_b) },
                { "method": "hew.solid.extrude", "as": "box", "params": {
                    "region": { "$ref": "profile#/region_id" },
                    "distance": distance
                }}
            ]
        }),
    );
    result["results"][1]["object_id"]
        .as_str()
        .unwrap()
        .to_string()
}

// ------------------------------------------------- refused transactions

/// A refused transaction leaves the serialized document byte-identical
/// (§6.1, §14): neither a kernel-typed refusal partway through nor a
/// `$ref` that fails to resolve at runtime (§6.2's `ref_resolution_failed`)
/// may leave any trace.
#[test]
fn refused_transactions_leave_the_document_byte_identical() {
    let mut conn = Connection::new(Grant::core(), "conformance");
    let mut doc = kernel::Document::new();
    hello_attach(&mut conn, &mut doc);

    // Prior content the refusals below must not disturb.
    build_box(
        &mut conn,
        &mut doc,
        2,
        [0.0, 0.0, 0.0],
        [0.5, 0.5, 0.0],
        0.3,
    );
    let before = doc.save();
    let undo_depth_before = doc.undo_depth();

    // Case 1: the transaction's SECOND command refuses with a kernel-typed
    // refusal (a bogus region id on the extrude).
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile",
                  "params": ground_rect([2.0, 2.0, 0.0], [2.5, 2.5, 0.0]) },
                { "method": "hew.solid.extrude",
                  "params": { "region": "rgn_deadbeef_1", "distance": 0.1 } }
            ]
        }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
    assert_eq!(data["failed_index"], 1);
    assert_eq!(data["failed_method"], "hew.solid.extrude");
    assert_eq!(
        doc.save(),
        before,
        "a refused transaction leaves the document untouched"
    );
    assert_eq!(
        doc.undo_depth(),
        undo_depth_before,
        "a refused transaction adds no undo entry"
    );

    // Case 2: the transaction's SECOND command is syntactically valid but
    // fails to resolve its `$ref` at RUNTIME (§6.2) — `draw_rect`'s result
    // carries no `curve_id` (that field is `draw_circle`'s), so pointing
    // at one is only knowable once the labeled command has actually run.
    let data = call_err(
        &mut conn,
        &mut doc,
        4,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile",
                  "params": ground_rect([3.0, 3.0, 0.0], [3.5, 3.5, 0.0]) },
                { "method": "hew.solid.extrude",
                  "params": { "region": { "$ref": "profile#/curve_id" }, "distance": 0.1 } }
            ]
        }),
    );
    assert_eq!(data["refusal"], "ref_resolution_failed");
    assert_eq!(data["failed_index"], 1);
    assert_eq!(
        doc.save(),
        before,
        "a $ref resolution failure leaves the document untouched"
    );
    assert_eq!(doc.undo_depth(), undo_depth_before);
}

// --------------------------------------------- one-envelope-one-undo carve-outs
//
// §14 calls for "the documented kernel gaps enumerated as an explicit,
// reviewed allowlist mirroring the kernel fuzz harnesses' posture"
// (crates/kernel/tests/op_fuzz.rs's `is_known_inverse_guard_gap` is that
// posture's kernel-side precedent). The task that commissioned this suite
// named three carve-outs (a)-(c); investigating (b) surfaced a fourth,
// (d), documented the same way. (b), (c), and (d) have since been fixed
// upstream in the kernel (the transform family recording exact pre-
// transform snapshots instead of a recomputed inverse, and
// `Sketch::remove_edges` freeing vertex/edge slots in descending id order
// so region-consuming undo lands geometry back in its original slots —
// see `crates/kernel/tests/transform_exact_specs.rs` and
// `region_consume_undo_reversal_specs.rs`) and folded into the
// byte-identity vocabulary below, leaving only (a).
//
// Carve-out (a) is not a kernel GAP like the fixed ones above — it is
// normatively specified behavior (docs/agents/HEW_API.md §6.4's registry-state
// paragraph), not an anomaly the spec merely tolerates. It is carved out
// of the same property for the same mechanical reason the real gaps were:
// the byte-identity vocabulary asserts exactly one undo entry per
// mutating envelope, and registry-state commands are specified to add
// none.

/// Carve-out (a): `hew.material.create` / `hew.tag.create` /
/// `hew.tag.set_visible` are the **registry-state** commands docs/agents/HEW_API.md
/// §6.4 specifies: model-mutating for placement purposes, but recording no
/// undo entry, because the kernel entry points behind them
/// (`Document::add_material`, `Document::set_tag_hidden`) deliberately keep
/// palette and registry additions outside the undo log — a palette addition
/// or a tag's hidden-by-default flag is view/registry state, not a modeled
/// edit. The one-envelope-one-undo property does not apply to these three —
/// they add ZERO undo entries, asserted here rather than assumed.
#[test]
fn material_and_tag_registry_writes_add_no_undo_entry() {
    let mut conn = Connection::new(Grant::core(), "conformance");
    let mut doc = kernel::Document::new();
    hello_attach(&mut conn, &mut doc);
    let depth = doc.undo_depth();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.material.create",
        json!({ "name": "Oak", "color": [180, 140, 90] }),
    );
    assert_eq!(
        doc.undo_depth(),
        depth,
        "hew.material.create must add no undo entry"
    );

    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.tag.create",
        json!({ "path": ["Structure"] }),
    );
    assert_eq!(
        doc.undo_depth(),
        depth,
        "hew.tag.create must add no undo entry"
    );

    call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.tag.set_visible",
        json!({ "path": ["Structure"], "visible": false }),
    );
    assert_eq!(
        doc.undo_depth(),
        depth,
        "hew.tag.set_visible must add no undo entry"
    );
}

// -------------------------------------- every_mutating_envelope_is_one_undo_entry

/// A small SAFE vocabulary for the byte-identity proptest below:
/// `draw_rect`, `draw_circle`, a draw-then-extrude-via-`$ref` transaction
/// (the §6.1 chaining shape), that same chain followed by a rotate or a
/// move of the just-extruded object (`$ref`-ing the extrude's own
/// `object_id`), and a draw-circle-then-draw-rect-then-follow_me chain —
/// every one of them undoes byte-identically now that the kernel's
/// transform family (`transform_object`/`transform_selection` and the
/// rest — see `crates/kernel/tests/transform_exact_specs.rs`) records an
/// exact pre-transform snapshot instead of baking a recomputed inverse, and
/// `Sketch::remove_edges` frees region-consumed vertex/edge slots in
/// descending id order so `follow_me`'s (and `extrude`'s)
/// `restore_edges`-based undo lands geometry back in its original slots
/// (see `crates/kernel/tests/region_consume_undo_reversal_specs.rs`).
/// Deliberately excludes the one remaining carve-out above
/// (`hew.material.create`/`hew.tag.create`/`hew.tag.set_visible`, which add
/// no undo entry at all) — see that carve-out test for the reason.
#[derive(Debug, Clone)]
enum SafeOp {
    RectOnly {
        ox: f64,
        oy: f64,
    },
    CircleOnly {
        ox: f64,
        oy: f64,
        radius: f64,
    },
    RectThenExtrude {
        ox: f64,
        oy: f64,
        distance: f64,
    },
    RectThenExtrudeThenRotate {
        ox: f64,
        oy: f64,
        distance: f64,
        angle: f64,
    },
    RectThenExtrudeThenMove {
        ox: f64,
        oy: f64,
        distance: f64,
        translation: (f64, f64, f64),
    },
    /// A circular path plus a square profile centered on the path's start
    /// point (radius, 0, 0), perpendicular to its tangent there — the same
    /// configuration as `commands_sketch_solid.rs`'s
    /// `follow_me_sweeps_a_square_profile_around_a_circular_path`, just
    /// translated by `(ox, oy)` and scaled by `radius` (the profile's own
    /// 0.02 half-extent is unaffected by `radius`, so it stays comfortably
    /// smaller than the path for every `radius` this strategy generates).
    CircleThenRectThenFollowMe {
        ox: f64,
        oy: f64,
        radius: f64,
    },
}

fn safe_op() -> impl Strategy<Value = SafeOp> {
    prop_oneof![
        (0.0..0.6f64, 0.0..0.6f64).prop_map(|(ox, oy)| SafeOp::RectOnly { ox, oy }),
        (0.0..0.6f64, 0.0..0.6f64, 0.1..0.3f64).prop_map(|(ox, oy, radius)| SafeOp::CircleOnly {
            ox,
            oy,
            radius
        }),
        (0.0..0.6f64, 0.0..0.6f64, 0.1..2.0f64)
            .prop_map(|(ox, oy, distance)| SafeOp::RectThenExtrude { ox, oy, distance }),
        (0.0..0.6f64, 0.0..0.6f64, 0.1..2.0f64, -3.0..3.0f64).prop_map(
            |(ox, oy, distance, angle)| SafeOp::RectThenExtrudeThenRotate {
                ox,
                oy,
                distance,
                angle
            }
        ),
        (
            0.0..0.6f64,
            0.0..0.6f64,
            0.1..2.0f64,
            -3.0..3.0f64,
            -3.0..3.0f64,
            -3.0..3.0f64
        )
            .prop_map(
                |(ox, oy, distance, dx, dy, dz)| SafeOp::RectThenExtrudeThenMove {
                    ox,
                    oy,
                    distance,
                    translation: (dx, dy, dz)
                }
            ),
        (0.0..0.6f64, 0.0..0.6f64, 0.05..0.2f64)
            .prop_map(|(ox, oy, radius)| { SafeOp::CircleThenRectThenFollowMe { ox, oy, radius } }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// §14: every successful model-mutating envelope adds exactly one undo
    /// entry, and undoing it restores `doc.save()` byte-for-byte — over
    /// small random command sequences drawn from the SAFE vocabulary above,
    /// each command dispatched as its own envelope (plain requests
    /// auto-wrap as one-command transactions per §6.1; `RectThenExtrude` is
    /// a two-command transaction, `RectThenExtrudeThenRotate`/
    /// `RectThenExtrudeThenMove`/`CircleThenRectThenFollowMe` three-command
    /// ones, all using the §6.1 `$ref` chaining shape).
    #[test]
    fn every_mutating_envelope_is_one_undo_entry(ops in prop::collection::vec(safe_op(), 1..6)) {
        let mut conn = Connection::new(Grant::core(), "conformance");
        let mut doc = kernel::Document::new();
        hello_attach(&mut conn, &mut doc);

        for (i, op) in ops.into_iter().enumerate() {
            let id = 1000 + i as i64;
            // Non-overlapping cells (mirrors transaction_specs.rs's
            // `i as f64 * 2.0` spacing convention) — sketches on the
            // ground plane are fresh per draw (§7's plane-spec semantics),
            // but well-separated cells keep every op's geometry legible
            // and independent regardless.
            let cell = i as f64 * 5.0;
            let before = doc.save();
            let depth_before = doc.undo_depth();

            match op {
                SafeOp::RectOnly { ox, oy } => {
                    call_ok(
                        &mut conn,
                        &mut doc,
                        id,
                        "hew.sketch.draw_rect",
                        ground_rect([cell + ox, oy, 0.0], [cell + ox + 0.4, oy + 0.4, 0.0]),
                    );
                }
                SafeOp::CircleOnly { ox, oy, radius } => {
                    call_ok(
                        &mut conn,
                        &mut doc,
                        id,
                        "hew.sketch.draw_circle",
                        json!({
                            "plane": { "ground": true },
                            "center": [cell + ox, oy, 0.0],
                            "radius": radius
                        }),
                    );
                }
                SafeOp::RectThenExtrude { ox, oy, distance } => {
                    call_ok(
                        &mut conn,
                        &mut doc,
                        id,
                        "hew.doc.transact",
                        json!({
                            "commands": [
                                { "method": "hew.sketch.draw_rect", "as": "profile",
                                  "params": ground_rect([cell + ox, oy, 0.0], [cell + ox + 0.4, oy + 0.4, 0.0]) },
                                { "method": "hew.solid.extrude", "params": {
                                    "region": { "$ref": "profile#/region_id" }, "distance": distance
                                }}
                            ]
                        }),
                    );
                }
                SafeOp::RectThenExtrudeThenRotate {
                    ox,
                    oy,
                    distance,
                    angle,
                } => {
                    call_ok(
                        &mut conn,
                        &mut doc,
                        id,
                        "hew.doc.transact",
                        json!({
                            "commands": [
                                { "method": "hew.sketch.draw_rect", "as": "profile",
                                  "params": ground_rect([cell + ox, oy, 0.0], [cell + ox + 0.4, oy + 0.4, 0.0]) },
                                { "method": "hew.solid.extrude", "as": "box", "params": {
                                    "region": { "$ref": "profile#/region_id" }, "distance": distance
                                }},
                                { "method": "hew.entity.rotate", "params": {
                                    "ids": [{ "$ref": "box#/object_id" }],
                                    "pivot": [cell + ox, oy, 0.0],
                                    "axis": [0.0, 0.0, 1.0],
                                    "angle": angle
                                }}
                            ]
                        }),
                    );
                }
                SafeOp::RectThenExtrudeThenMove {
                    ox,
                    oy,
                    distance,
                    translation: (dx, dy, dz),
                } => {
                    call_ok(
                        &mut conn,
                        &mut doc,
                        id,
                        "hew.doc.transact",
                        json!({
                            "commands": [
                                { "method": "hew.sketch.draw_rect", "as": "profile",
                                  "params": ground_rect([cell + ox, oy, 0.0], [cell + ox + 0.4, oy + 0.4, 0.0]) },
                                { "method": "hew.solid.extrude", "as": "box", "params": {
                                    "region": { "$ref": "profile#/region_id" }, "distance": distance
                                }},
                                { "method": "hew.entity.move", "params": {
                                    "ids": [{ "$ref": "box#/object_id" }],
                                    "translation": [dx, dy, dz]
                                }}
                            ]
                        }),
                    );
                }
                SafeOp::CircleThenRectThenFollowMe { ox, oy, radius } => {
                    let path_center = [cell + ox, oy, 0.0];
                    let seam = cell + ox + radius; // path's start point, (radius, 0, 0) from its center
                    call_ok(
                        &mut conn,
                        &mut doc,
                        id,
                        "hew.doc.transact",
                        json!({
                            "commands": [
                                { "method": "hew.sketch.draw_circle", "as": "path", "params": {
                                    "plane": { "ground": true },
                                    "center": path_center,
                                    "radius": radius
                                }},
                                { "method": "hew.sketch.draw_rect", "as": "profile", "params": {
                                    "plane": { "origin": [seam, oy, 0.0], "normal": [0.0, 1.0, 0.0] },
                                    "corner_a": [seam - 0.02, oy, -0.02],
                                    "corner_b": [seam + 0.02, oy, 0.02]
                                }},
                                { "method": "hew.solid.follow_me", "params": {
                                    "profile": { "$ref": "profile#/region_id" },
                                    "path": { "curve": { "$ref": "path#/curve_id" } }
                                }}
                            ]
                        }),
                    );
                }
            }

            prop_assert_eq!(
                doc.undo_depth(),
                depth_before + 1,
                "envelope should add exactly one undo entry"
            );
            doc.undo().expect("undo succeeds");
            prop_assert_eq!(doc.save(), before, "undo should restore byte-identical bytes");
            doc.redo().expect("redo succeeds"); // keep building for the next op
        }
    }
}

// -------------------------------------------------------- derived points

/// Derived points (§5.3) resolve to the kernel's own exact `f64` values —
/// midpoint, endpoint, center, and bbox against a known box (corners at
/// integer coordinates, so every expected value below is exactly
/// representable) plus a known circle sketch; `hew.query.measure` between
/// two of them carries the same exactness through its `delta`. A face
/// locator sitting exactly on the shared edge of two faces refuses
/// `ambiguous_locator` rather than guessing (§5.2).
#[test]
fn derived_points_resolve_exactly_and_ambiguity_refuses() {
    let mut conn = Connection::new(Grant::core(), "conformance");
    let mut doc = kernel::Document::new();
    hello_attach(&mut conn, &mut doc);

    // A 4x2x1 box at the origin: every vertex, edge endpoint, and bbox
    // corner is an exact small integer.
    let object_id = build_box(
        &mut conn,
        &mut doc,
        2,
        [0.0, 0.0, 0.0],
        [4.0, 2.0, 0.0],
        1.0,
    );
    // A loose (never-extruded) circle sketch for the `center` derived point.
    let circle = single(call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_circle",
        json!({ "plane": { "ground": true }, "center": [10.0, 10.0, 0.0], "radius": 1.5 }),
    ));
    let curve_id = circle["curve_id"].as_str().unwrap().to_string();

    // midpoint: the bottom-front edge runs (0,0,0)-(4,0,0); its exact
    // midpoint is (2,0,0). `at` picks the edge via a point strictly on it.
    let midpoint = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.query.resolve",
        json!({ "point": { "point": "midpoint", "of": { "edge": { "object": object_id, "at": [1.0, 0.0, 0.0] } } } }),
    );
    assert_eq!(midpoint["point"], json!([2.0, 0.0, 0.0]));

    // endpoint: the same edge's endpoint nearest the origin is (0,0,0).
    let endpoint = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.query.resolve",
        json!({
            "point": { "point": "endpoint",
                "of": { "edge": { "object": object_id, "at": [1.0, 0.0, 0.0] } },
                "nearest": [0.0, 0.0, 0.0] }
        }),
    );
    assert_eq!(endpoint["point"], json!([0.0, 0.0, 0.0]));

    // center: the circle's stored analytic center, read back verbatim.
    let center = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.query.resolve",
        json!({ "point": { "point": "center", "of": curve_id } }),
    );
    assert_eq!(center["point"], json!([10.0, 10.0, 0.0]));

    // bbox: min, max, and their exact midpoint as "center".
    let bbox_min = call_ok(
        &mut conn,
        &mut doc,
        7,
        "hew.query.resolve",
        json!({ "point": { "point": "bbox", "of": object_id, "anchor": "min" } }),
    );
    assert_eq!(bbox_min["point"], json!([0.0, 0.0, 0.0]));
    let bbox_max = call_ok(
        &mut conn,
        &mut doc,
        8,
        "hew.query.resolve",
        json!({ "point": { "point": "bbox", "of": object_id, "anchor": "max" } }),
    );
    assert_eq!(bbox_max["point"], json!([4.0, 2.0, 1.0]));
    let bbox_center = call_ok(
        &mut conn,
        &mut doc,
        9,
        "hew.query.resolve",
        json!({ "point": { "point": "bbox", "of": object_id, "anchor": "center" } }),
    );
    assert_eq!(bbox_center["point"], json!([2.0, 1.0, 0.5]));

    // measure: bbox min -> the bottom-front edge's midpoint. delta AND
    // distance both land on exact integers here (2, 0, 0 / 2.0).
    let measured = call_ok(
        &mut conn,
        &mut doc,
        10,
        "hew.query.measure",
        json!({
            "from": { "point": "bbox", "of": object_id, "anchor": "min" },
            "to": { "point": "midpoint", "of": { "edge": { "object": object_id, "at": [1.0, 0.0, 0.0] } } }
        }),
    );
    assert_eq!(measured["delta"], json!([2.0, 0.0, 0.0]));
    assert_eq!(measured["distance"], json!(2.0));

    // Ambiguity: (1, 0, 0) sits exactly on the shared edge of the front
    // face (y=0) and the bottom face (z=0) — a face locator there refuses
    // typed rather than guessing which face was meant.
    let ambiguous = call_err(
        &mut conn,
        &mut doc,
        11,
        "hew.query.resolve",
        json!({ "face": { "object": object_id, "at": [1.0, 0.0, 0.0] } }),
    );
    assert_eq!(ambiguous["refusal"], "ambiguous_locator");
}

// ------------------------------------------------------- determinism replay

/// A script of envelopes (hello -> attach -> draw -> extrude -> push_pull)
/// replayed headlessly through two fresh `Connection` + `Document` pairs
/// produces byte-identical `.hew` bytes — the API inherits the kernel's
/// replay determinism guarantee (§2, §14).
#[test]
fn envelope_scripts_replay_byte_identically() {
    fn run_script() -> Vec<u8> {
        let mut conn = Connection::new(Grant::core(), "conformance");
        let mut doc = kernel::Document::new();
        hello_attach(&mut conn, &mut doc);
        let result = call_ok(
            &mut conn,
            &mut doc,
            2,
            "hew.doc.transact",
            json!({
                "label": "Table leg",
                "commands": [
                    { "method": "hew.sketch.draw_rect", "as": "profile",
                      "params": ground_rect([0.0, 0.0, 0.0], [0.2, 0.2, 0.0]) },
                    { "method": "hew.solid.extrude", "as": "leg", "params": {
                        "region": { "$ref": "profile#/region_id" }, "distance": 0.45
                    }},
                    { "method": "hew.solid.push_pull", "as": "pushed", "params": {
                        "face": { "$face": "leg#top" }, "distance": 0.05
                    }}
                ]
            }),
        );
        assert!(result["results"][2]["object_id"].as_str().is_some());
        doc.save()
    }

    let first = run_script();
    let second = run_script();
    assert_eq!(
        first, second,
        "the same envelope script must replay to byte-identical bytes"
    );
}

// -------------------------------------------------------- registry completeness

/// §14's registry-completeness check, extended over the dispatch surface
/// itself (registry.rs's own `every_declaration_is_complete` covers
/// schemas/summaries/refusal inventories): every command the registry
/// marks `implemented: true` has a `commands::handler` entry, and every
/// `implemented: false` command has none — with `hew.meta.hello`,
/// `hew.meta.capabilities`, `hew.doc.attach`, and `hew.doc.transact`
/// allowlisted, because dispatch.rs's `reply` routes those four INLINE
/// (never through `commands::handler`) regardless of their registry flag.
///
/// `hew.doc.transact` is allowlisted regardless of its flag (now
/// truthfully `implemented: true`) because it, too, never routes through
/// the handler table.
#[test]
fn every_implemented_command_has_a_handler_and_vice_versa() {
    const DISPATCH_INLINE: [&str; 6] = [
        "hew.meta.hello",
        "hew.meta.capabilities",
        "hew.doc.attach",
        "hew.doc.transact",
        "hew.event.subscribe",
        "hew.event.unsubscribe",
    ];
    let registry = api::Registry::protocol_1();
    // Plugin commands never reach `commands::handler`: dispatch forwards
    // them (`plugin_commands_have_no_handler`).
    for cmd in registry.commands().filter(|c| !c.is_plugin()) {
        if DISPATCH_INLINE.contains(&cmd.name.as_str()) {
            assert!(
                api::commands::handler(&cmd.name).is_none(),
                "{} is dispatch-inline and should carry no commands::handler entry",
                cmd.name
            );
            continue;
        }
        let has_handler = api::commands::handler(&cmd.name).is_some();
        assert_eq!(
            has_handler, cmd.implemented,
            "{}: registry implemented={} but commands::handler().is_some()={has_handler}",
            cmd.name, cmd.implemented
        );
    }
}

/// First-party capabilities of first-party commands are byte-identical
/// with or without a plugin registered; a plugin entry carries exactly the
/// six forwarding members on top of the usual eight.
#[test]
fn capabilities_marks_plugin_entries_and_is_unchanged_for_first_party() {
    let capabilities = |registry: &api::Registry| -> Value {
        let mut conn = Connection::new(Grant::core(), "conformance");
        let mut doc = kernel::Document::new();
        for (id, method) in [(0, "hew.meta.hello"), (1, "hew.meta.capabilities")] {
            let request: Request = serde_json::from_value(json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": { "protocol": 1 }
            }))
            .unwrap();
            if let DispatchOutcome::Reply(r) =
                conn.dispatch(registry, &mut doc, &mut api::NoHost, request)
                && id == 1
            {
                return r.result.expect("capabilities succeeds");
            }
        }
        unreachable!()
    };
    let bare = capabilities(&api::Registry::protocol_1());
    let mut with_plugin = api::Registry::protocol_1();
    let decls: Vec<plugin_manifest::CommandContribution> = serde_json::from_value(json!([
        { "id": "shelf.add", "class": "mutating", "summary": "Adds a shelf.", "label": "Add shelf", "concurrency": "serial", "timeout_ms": 1000 }
    ]))
    .unwrap();
    with_plugin
        .register_plugin("com.example.lib", &decls)
        .unwrap();
    let marked = capabilities(&with_plugin);

    let first_party: Vec<&Value> = marked["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("provider").is_none())
        .collect();
    assert_eq!(
        serde_json::to_string(&first_party).unwrap(),
        serde_json::to_string(bare["commands"].as_array().unwrap()).unwrap(),
        "first-party entries are byte-identical"
    );
    let plugin = marked["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "com.example.lib.shelf.add")
        .expect("the plugin command is listed");
    assert_eq!(plugin["provider"], json!({ "plugin": "com.example.lib" }));
    assert_eq!(plugin["permission"], "model.write");
    assert_eq!(plugin["transactable"], false);
    assert_eq!(plugin["label"], "Add shelf");
    assert_eq!(plugin["timeout_ms"], 1000);
    assert_eq!(plugin["concurrency"], "serial");
    assert_eq!(plugin["class"], "solitary");
    assert_eq!(plugin.as_object().unwrap().len(), 14);
}

/// A registered plugin's commands have no `commands::handler` entry, and
/// the first-party handler check above skips them.
#[test]
fn plugin_commands_have_no_handler() {
    let mut registry = api::Registry::protocol_1();
    let decls: Vec<plugin_manifest::CommandContribution> = serde_json::from_value(json!([
        { "id": "sum", "class": "read_only", "summary": "Adds." },
        { "id": "shelf.add", "class": "mutating", "summary": "Adds a shelf." }
    ]))
    .unwrap();
    registry.register_plugin("com.example.lib", &decls).unwrap();
    let names: Vec<&str> = registry
        .plugin_commands("com.example.lib")
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["com.example.lib.shelf.add", "com.example.lib.sum"]);
    for name in names {
        assert!(api::commands::handler(name).is_none(), "{name}");
    }
}

#[test]
fn structural_equality_is_bitwise_for_floats() {
    let a = serde_json::json!({"x": 0.0});
    let b = serde_json::json!({"x": -0.0});
    assert!(
        !structurally_equal(&a, &b),
        "0.0 and -0.0 are different bits"
    );
    assert!(structurally_equal(&a, &a.clone()));
    let int_one = serde_json::json!(1);
    let float_one = serde_json::json!(1.0);
    assert!(
        !structurally_equal(&int_one, &float_one),
        "integer 1 and float 1.0 are different serializations"
    );
}

// ------------------------------------------------- final-review regressions
//
// Each pins a defect the closing adversarial review confirmed; none may
// be weakened (rule 5).

/// §6.4/§13: a solitary command IS reachable as the sole command of a
/// transact envelope — its canonical MCP invocation — and runs bare (no
/// undo entry).
#[test]
fn single_solitary_transact_reaches_the_command() {
    let mut conn = Connection::new(Grant::core(), "conformance");
    let mut doc = kernel::Document::new();
    let mut host = api::NoHost;
    for frame in [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}),
    ] {
        let request: Request = serde_json::from_value(frame).unwrap();
        conn.dispatch(&REGISTRY, &mut doc, &mut host, request);
    }
    let request: Request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0","id":3,"method":"hew.doc.transact",
        "params":{"commands":[{"method":"hew.history.status","params":{}}]}
    }))
    .unwrap();
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, &mut doc, &mut host, request) else {
        panic!()
    };
    assert!(
        r.error.is_none(),
        "solitary-as-sole-command succeeds: {:?}",
        r.error
    );
    let result = r.result.unwrap();
    assert_eq!(result["results"][0]["undo_depth"], 0);
    assert_eq!(doc.undo_depth(), 0, "solitary envelopes add no undo entry");

    // Two solitary commands together still reject statically.
    let request: Request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0","id":4,"method":"hew.doc.transact",
        "params":{"commands":[
            {"method":"hew.history.status","params":{}},
            {"method":"hew.history.status","params":{}}
        ]}
    }))
    .unwrap();
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, &mut doc, &mut host, request) else {
        panic!()
    };
    assert_eq!(r.error.unwrap().code, -32602);
}

/// §6.2: a `$ref` object carries exactly that one key — sibling keys are
/// a static defect, never a silent partial substitution.
#[test]
fn ref_with_sibling_keys_rejects_statically() {
    let mut conn = Connection::new(Grant::core(), "conformance");
    let mut doc = kernel::Document::new();
    let mut host = api::NoHost;
    for frame in [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}),
    ] {
        let request: Request = serde_json::from_value(frame).unwrap();
        conn.dispatch(&REGISTRY, &mut doc, &mut host, request);
    }
    let bytes_before = doc.save();
    let request: Request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0","id":3,"method":"hew.doc.transact",
        "params":{"commands":[
            {"method":"hew.sketch.draw_rect","as":"r",
             "params":{"plane":{"ground":true},"corner_a":[0.0,0.0,0.0],"corner_b":[1.0,1.0,0.0]}},
            {"method":"hew.solid.extrude",
             "params":{"region":{"$ref":"r#/region_id","bogus":1},"distance":0.5}}
        ]}
    }))
    .unwrap();
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, &mut doc, &mut host, request) else {
        panic!()
    };
    assert_eq!(r.error.unwrap().code, -32602);
    assert_eq!(doc.save(), bytes_before, "static rejection ran nothing");
}

/// A huge-sweep arc with no explicit segment count stays capped — the
/// default facet count obeys MAX_CIRCLE_SEGMENTS exactly like an
/// explicit one, and a huge array-copy count refuses typed.
#[test]
fn arc_default_facets_and_array_counts_are_capped() {
    let mut conn = Connection::new(Grant::core(), "conformance");
    let mut doc = kernel::Document::new();
    let mut host = api::NoHost;
    for frame in [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}),
    ] {
        let request: Request = serde_json::from_value(frame).unwrap();
        conn.dispatch(&REGISTRY, &mut doc, &mut host, request);
    }
    // 10,000 full turns of sweep: rejected promptly — wrapped chords
    // would cross pairwise and hang the sketch's intersection graph.
    let request: Request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0","id":3,"method":"hew.sketch.draw_arc",
        "params":{"plane":{"ground":true},"center":[0.0,0.0,0.0],"radius":1.0,
                   "start_angle":0.0,"end_angle":62831.853071795864}
    }))
    .unwrap();
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, &mut doc, &mut host, request) else {
        panic!()
    };
    assert_eq!(r.error.unwrap().code, -32602, "over-turn sweep rejects");

    // A legitimate three-quarter-turn arc with default segments succeeds.
    let request: Request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0","id":30,"method":"hew.sketch.draw_arc",
        "params":{"plane":{"ground":true},"center":[0.0,0.0,0.0],"radius":1.0,
                   "start_angle":0.0,"end_angle":4.71238898038469}
    }))
    .unwrap();
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, &mut doc, &mut host, request) else {
        panic!()
    };
    assert!(
        r.error.is_none(),
        "3/4-turn default arc draws: {:?}",
        r.error
    );

    // Array copy count beyond the cap refuses typed.
    let draw: Request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0","id":4,"method":"hew.doc.transact",
        "params":{"commands":[
            {"method":"hew.sketch.draw_rect","as":"r",
             "params":{"plane":{"ground":true},"corner_a":[10.0,0.0,0.0],"corner_b":[11.0,1.0,0.0]}},
            {"method":"hew.solid.extrude","as":"leg",
             "params":{"region":{"$ref":"r#/region_id"},"distance":0.5}}
        ]}
    }))
    .unwrap();
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, &mut doc, &mut host, draw) else {
        panic!()
    };
    let object = r.result.unwrap()["results"][1]["object_id"]
        .as_str()
        .unwrap()
        .to_string();
    let request: Request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0","id":5,"method":"hew.entity.move",
        "params":{"ids":[object],"translation":[2.0,0.0,0.0],"copy":{"count":100000}}
    }))
    .unwrap();
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, &mut doc, &mut host, request) else {
        panic!()
    };
    let err = r.error.unwrap();
    assert_eq!(err.code, -32000);
    assert_eq!(err.data.unwrap()["refusal"], "array_count_too_large");

    // Empty selection refuses with its declared machine name.
    let request: Request = serde_json::from_value(serde_json::json!({
        "jsonrpc":"2.0","id":6,"method":"hew.entity.move",
        "params":{"ids":[],"translation":[1.0,0.0,0.0]}
    }))
    .unwrap();
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, &mut doc, &mut host, request) else {
        panic!()
    };
    let err = r.error.unwrap();
    assert_eq!(err.code, -32000);
    assert_eq!(err.data.unwrap()["refusal"], "empty_selection");
}
