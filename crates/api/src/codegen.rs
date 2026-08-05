//! Generators for the two remaining docs/HEW_API.md §9 artifacts: the
//! TypeScript client SDK (`app/src/api/hewApi.gen.ts`) and the published
//! API reference (`docs/API_REFERENCE.gen.md`). Both are driven, at
//! runtime, off [`Registry::protocol_1`] — no TypeScript parsing, no
//! hand-maintained copies of a command's contract (§9: "hand-maintained
//! copies of a command's contract are forbidden because they will
//! drift").
//!
//! `crates/api/tests/generate_artifacts.rs` is the REGENERATE harness that
//! calls these two functions and either asserts byte-identity against the
//! committed files (plain `cargo test -p api`) or writes them
//! (`REGENERATE_API_ARTIFACTS=1`) — the same posture as
//! `refusal_copy.gen.rs`'s own generator
//! (`app/src/kernelErrorsDump.test.ts`).
//!
//! Determinism (§14: "the generated artifacts … regenerate
//! byte-identically in CI") falls out of the registry's own shape: it is
//! a `BTreeMap` keyed by command name, and every `serde_json::Value` here
//! is built without the `preserve_order` feature, so nested object keys
//! are `BTreeMap`-ordered too. No timestamps, no hash-map iteration.

use crate::refusal::{explanation_for, pascal_case};
use crate::registry::{CommandClass, CommandDecl, Registry, Served, Tier};
use serde_json::Value;
use std::collections::BTreeMap;

const REGEN_COMMAND: &str =
    "REGENERATE_API_ARTIFACTS=1 cargo test -p api --test generate_artifacts";

// ------------------------------------------------------------- naming

/// `"hew.sketch.draw_rect"` → `("sketch", "draw_rect")`. Every registry
/// name is `hew.<namespace>.<command>` (enforced by
/// `registry::tests::every_declaration_is_complete`), so this never sees
/// a name with a different shape.
fn split_command(name: &str) -> (&str, &str) {
    let mut parts = name.splitn(3, '.');
    parts.next(); // "hew"
    let ns = parts.next().unwrap_or_default();
    let cmd = parts.next().unwrap_or_default();
    (ns, cmd)
}

/// `push_pull` → `pushPull` — the client method name within its
/// namespace object.
fn camel_case(snake: &str) -> String {
    let pascal = pascal_case(snake);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `("sketch", "draw_rect")` → `"SketchDrawRect"` — the shared stem for a
/// command's `Params`/`Result` type names.
fn type_stem(ns: &str, cmd: &str) -> String {
    format!("{}{}", pascal_case(ns), pascal_case(cmd))
}

// -------------------------------------------------- JSON Schema → TS

/// Wraps a union type in parens before appending an array/tuple suffix,
/// so `(A | B)[]` doesn't silently parse as `A | B[]`.
fn parenthesize_union(ty: &str) -> String {
    if ty.contains(" | ") {
        format!("({ty})")
    } else {
        ty.to_string()
    }
}

/// Renders a JSON Schema fragment as a TypeScript type expression —
/// always inline (never a named declaration; callers that want a name
/// wrap the result in `export type X = …` or, for a plain object shape,
/// prefer [`write_named_type`]'s `interface` form).
///
/// - `oneOf` → a union of the rendered variants.
/// - `enum` → a union of string-literal types.
/// - `type: "object"` with `properties` → an inline `{ k: T; k2?: T2 }`
///   object type, `?` per the schema's `required` list.
/// - `type: "object"` with no `properties` (the registry's scaffold
///   placeholder, and several deliberately-generic locator/point
///   schemas) → `UnspecifiedShape`, the shared permissive alias.
/// - `type` as an array (e.g. `["string", "null"]`) → a union of the
///   mapped primitives.
/// - `type: "array"` with equal `minItems`/`maxItems` → a fixed-length
///   tuple; otherwise `T[]`.
/// - No recognizable shape at all (a bare `{}`) → `UnspecifiedShape`.
fn schema_to_ts(schema: &Value) -> String {
    if let Some(variants) = schema.get("oneOf").and_then(Value::as_array) {
        return variants
            .iter()
            .map(schema_to_ts)
            .collect::<Vec<_>>()
            .join(" | ");
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values
            .iter()
            .map(|v| match v {
                Value::String(_) => serde_json::to_string(v).expect("string serializes"),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" | ");
    }
    match schema.get("type") {
        Some(Value::String(t)) => match t.as_str() {
            "string" => "string".to_string(),
            "number" | "integer" => "number".to_string(),
            "boolean" => "boolean".to_string(),
            "null" => "null".to_string(),
            "array" => {
                let item_ty = schema
                    .get("items")
                    .map(schema_to_ts)
                    .unwrap_or_else(|| "unknown".to_string());
                let min = schema.get("minItems").and_then(Value::as_u64);
                let max = schema.get("maxItems").and_then(Value::as_u64);
                if let (Some(min), Some(max)) = (min, max)
                    && min == max
                    && (1..=6).contains(&min)
                {
                    return format!("[{}]", vec![item_ty; min as usize].join(", "));
                }
                format!("{}[]", parenthesize_union(&item_ty))
            }
            "object" => match schema.get("properties").and_then(Value::as_object) {
                Some(props) => render_inline_object(props, schema.get("required")),
                None => "UnspecifiedShape".to_string(),
            },
            _ => "unknown".to_string(),
        },
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .map(|t| match t {
                "string" => "string".to_string(),
                "number" | "integer" => "number".to_string(),
                "boolean" => "boolean".to_string(),
                "null" => "null".to_string(),
                _ => "unknown".to_string(),
            })
            .collect::<Vec<_>>()
            .join(" | "),
        _ => "UnspecifiedShape".to_string(),
    }
}

/// One field of an inline object type or a top-level interface.
struct Field {
    name: String,
    optional: bool,
    ty: String,
    doc: Option<String>,
}

fn fields_of(props: &serde_json::Map<String, Value>, required: Option<&Value>) -> Vec<Field> {
    let required: Vec<&str> = required
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    props
        .iter()
        .map(|(name, schema)| Field {
            name: name.clone(),
            optional: !required.contains(&name.as_str()),
            ty: schema_to_ts(schema),
            doc: schema
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
        .collect()
}

/// Renders an object-with-properties schema inline: `{ k: T; k2?: T2 }`.
fn render_inline_object(
    props: &serde_json::Map<String, Value>,
    required: Option<&Value>,
) -> String {
    let fields = fields_of(props, required);
    if fields.is_empty() {
        return "{}".to_string();
    }
    let body = fields
        .iter()
        .map(|f| format!("{}{}: {}", f.name, if f.optional { "?" } else { "" }, f.ty))
        .collect::<Vec<_>>()
        .join("; ");
    format!("{{ {body} }}")
}

/// Writes one field of a named interface, one line, with a leading
/// `/** … */` doc comment when the schema carried a `description`.
fn write_field(out: &mut String, indent: &str, f: &Field) {
    if let Some(doc) = &f.doc {
        out.push_str(indent);
        out.push_str("/** ");
        out.push_str(doc);
        out.push_str(" */\n");
    }
    out.push_str(indent);
    out.push_str(&f.name);
    if f.optional {
        out.push('?');
    }
    out.push_str(": ");
    out.push_str(&f.ty);
    out.push('\n');
}

/// Emits a command's params/result schema as a named TS declaration:
/// an `export interface Name { … }` for a plain object-with-properties
/// schema (the common, idiomatic case), or `export type Name = …` for
/// everything else (unions, arrays, the permissive alias).
fn write_named_type(out: &mut String, name: &str, schema: &Value) {
    let is_plain_object = schema.get("oneOf").is_none()
        && schema.get("type").and_then(Value::as_str) == Some("object")
        && schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some();
    if is_plain_object {
        let props = schema.get("properties").and_then(Value::as_object).unwrap();
        let fields = fields_of(props, schema.get("required"));
        if fields.is_empty() {
            out.push_str(&format!("export interface {name} {{}}\n\n"));
            return;
        }
        out.push_str(&format!("export interface {name} {{\n"));
        for f in &fields {
            write_field(out, "  ", f);
        }
        out.push_str("}\n\n");
        return;
    }
    out.push_str(&format!(
        "export type {name} = {}\n\n",
        schema_to_ts(schema)
    ));
}

// ----------------------------------------------------- shared labels

fn tier_label(tier: Tier) -> &'static str {
    match tier {
        Tier::Required => "Required",
        Tier::Standard => "Standard",
    }
}

fn class_label(class: CommandClass) -> &'static str {
    match class {
        CommandClass::ModelMutating => "model-mutating",
        CommandClass::ReadOnly => "read-only",
        CommandClass::Solitary => "solitary",
    }
}

fn served_label(served: Served) -> &'static str {
    match served {
        Served::Kernel => "kernel",
        Served::Host => "host",
    }
}

/// Renders a `/** … */` TSDoc block from plain-text lines. A plain Rust
/// string literal's backslash-newline continuation strips ALL leading
/// whitespace off the next line (not just the source indentation) — that
/// silently eats the intended `" * "` gutter on every continuation line
/// if built by hand, so every multi-line doc comment in this generator
/// goes through here instead.
fn doc_comment(lines: &[&str]) -> String {
    let mut s = String::new();
    for (i, line) in lines.iter().enumerate() {
        s.push_str(if i == 0 { "/** " } else { " * " });
        s.push_str(line);
        s.push_str(if i + 1 == lines.len() { " */\n" } else { "\n" });
    }
    s
}

// ------------------------------------------------------- TS SDK

/// Generates `app/src/api/hewApi.gen.ts` — a typed client over a
/// caller-supplied `HewTransport`-shaped transport (docs/HEW_API.md
/// §9). One `Params`/`Result` type pair per command, one typed method
/// per command grouped by namespace on `HewApiClient`, plus the
/// error-code constants and the canonical §4.4 refusal shape.
pub fn generate_ts_sdk(registry: &Registry) -> String {
    let mut out = String::new();

    out.push_str("// GENERATED from crates/api registry — do not edit; regenerate with:\n");
    out.push_str(&format!("//   {REGEN_COMMAND}\n"));
    out.push_str("//\n");
    out.push_str(
        "// A typed client over a caller-supplied transport for the Hew API\n\
         // (docs/HEW_API.md — the normative reference; §9 is this file's\n\
         // contract). Every `Params`/`Result` pair and every method below is\n\
         // derived mechanically from the command registry in `crates/api`,\n\
         // which is their single source of truth — this file has no\n\
         // hand-maintained copy of a command's shape.\n",
    );
    out.push('\n');

    out.push_str(&doc_comment(&[
        "One JSON-RPC 2.0 request frame (docs/HEW_API.md §4.1). `params` is",
        "always a single object, never positional.",
    ]));
    out.push_str("export interface JsonRpcRequest {\n");
    out.push_str("  jsonrpc: '2.0'\n");
    out.push_str("  id: number | string\n");
    out.push_str("  method: string\n");
    out.push_str("  params?: unknown\n");
    out.push_str("}\n\n");

    out.push_str(&doc_comment(&[
        "One JSON-RPC 2.0 response frame: exactly one of `result` / `error`",
        "is present (§4.1).",
    ]));
    out.push_str("export interface JsonRpcResponse {\n");
    out.push_str("  jsonrpc: '2.0'\n");
    out.push_str("  id: number | string | null\n");
    out.push_str("  result?: unknown\n");
    out.push_str("  error?: JsonRpcErrorObject\n");
    out.push_str("}\n\n");

    out.push_str("/** A JSON-RPC 2.0 error object (§4.4). */\n");
    out.push_str("export interface JsonRpcErrorObject {\n");
    out.push_str("  code: number\n");
    out.push_str("  message: string\n");
    out.push_str("  data?: unknown\n");
    out.push_str("}\n\n");

    out.push_str(&doc_comment(&[
        "The transport this client dispatches every envelope through — an",
        "in-process call, the desktop app's local socket, or `hew-cli`'s",
        "stdio MCP adapter all satisfy this one shape (docs/HEW_API.md",
        "§11). The client owns request framing (`id` assignment) and",
        "result/error unwrapping; the transport owns only the wire.",
    ]));
    out.push_str("export interface HewTransport {\n");
    out.push_str("  dispatch(request: JsonRpcRequest): Promise<JsonRpcResponse>\n");
    out.push_str("}\n\n");

    out.push_str("/** The protocol's error-code inventory (§4.4). Additive only (§9). */\n");
    out.push_str("export const HewErrorCode = {\n");
    out.push_str("  PARSE_ERROR: -32700,\n");
    out.push_str("  METHOD_NOT_FOUND: -32601,\n");
    out.push_str("  INVALID_PARAMS: -32602,\n");
    out.push_str("  REFUSED: -32000,\n");
    out.push_str("  NOT_PERMITTED: -32001,\n");
    out.push_str("  NO_DOCUMENT: -32002,\n");
    out.push_str("  INTERNAL_FAULT: -32003,\n");
    out.push_str("  NOT_READY: -32004,\n");
    out.push_str("} as const\n\n");
    out.push_str("export type HewErrorCode = (typeof HewErrorCode)[keyof typeof HewErrorCode]\n\n");

    out.push_str(&doc_comment(&[
        "The canonical `error.data` shape of a refusal (`code ===",
        "HewErrorCode.REFUSED`) — §4.4: always all five fields, whether the",
        "envelope was a transaction or a plain request.",
    ]));
    out.push_str("export interface HewRefusal {\n");
    out.push_str("  refusal: string\n");
    out.push_str("  failed_index: number\n");
    out.push_str("  failed_method: string\n");
    out.push_str("  detail: unknown\n");
    out.push_str("  explanation: string\n");
    out.push_str("}\n\n");

    out.push_str(&doc_comment(&[
        "Thrown by every generated method when the dispatcher answers an",
        "error frame. `refusal` is populated (typed as {@link HewRefusal})",
        "exactly when `code === HewErrorCode.REFUSED`; other codes are",
        "protocol errors or internal faults (§4.4) and carry `data` as-is,",
        "if any.",
    ]));
    out.push_str("export class HewApiError extends Error {\n");
    out.push_str("  readonly code: number\n");
    out.push_str("  readonly data: unknown\n");
    out.push_str("  readonly refusal: HewRefusal | undefined\n\n");
    out.push_str("  constructor(error: JsonRpcErrorObject) {\n");
    out.push_str("    super(error.message)\n");
    out.push_str("    this.name = 'HewApiError'\n");
    out.push_str("    this.code = error.code\n");
    out.push_str("    this.data = error.data\n");
    out.push_str(
        "    this.refusal = error.code === HewErrorCode.REFUSED ? (error.data as HewRefusal) : undefined\n",
    );
    out.push_str("  }\n");
    out.push_str("}\n\n");

    out.push_str(&doc_comment(&[
        "A command whose registry schema is still the scaffold placeholder",
        "(an untightened `{\"type\": \"object\"}` — docs/HEW_API.md §14's",
        "burn-down posture) falls back to this rather than a fabricated",
        "shape.",
    ]));
    out.push_str("export type UnspecifiedShape = Record<string, unknown>\n\n");

    // ---- per-command types, grouped by namespace in registry order ----
    let mut namespaces: BTreeMap<&str, Vec<&CommandDecl>> = BTreeMap::new();
    for cmd in registry.commands() {
        let (ns, _) = split_command(cmd.name);
        namespaces.entry(ns).or_default().push(cmd);
    }

    for cmd in registry.commands() {
        let (ns, name) = split_command(cmd.name);
        let stem = type_stem(ns, name);
        out.push_str(&format!(
            "/**\n * `{}` (v{}) — {}\n * Tier: {} · Class: {} · Served: {}\n",
            cmd.name,
            cmd.version,
            cmd.summary,
            tier_label(cmd.tier),
            class_label(cmd.class),
            served_label(cmd.served),
        ));
        if cmd.refusals.is_empty() {
            out.push_str(" * Refusals: none.\n");
        } else {
            out.push_str(&format!(" * Refusals: {}\n", cmd.refusals.join(", ")));
        }
        out.push_str(" */\n");
        write_named_type(&mut out, &format!("{stem}Params"), &cmd.params_schema);
        write_named_type(&mut out, &format!("{stem}Result"), &cmd.result_schema);
    }

    // ---- the client, namespace-grouped ----
    out.push_str(&doc_comment(&[
        "A typed client over a caller-supplied {@link HewTransport}. One",
        "method per registry command, grouped by namespace",
        "(`client.sketch.drawRect(…)`), generated from the registry above.",
    ]));
    out.push_str("export class HewApiClient {\n");
    out.push_str("  private nextId = 1\n\n");
    out.push_str("  constructor(private readonly transport: HewTransport) {}\n\n");
    out.push_str(
        "  private async call<TResult>(method: string, params: unknown): Promise<TResult> {\n",
    );
    out.push_str("    const request: JsonRpcRequest = {\n");
    out.push_str("      jsonrpc: '2.0',\n");
    out.push_str("      id: this.nextId++,\n");
    out.push_str("      method,\n");
    out.push_str("      params,\n");
    out.push_str("    }\n");
    out.push_str("    const response = await this.transport.dispatch(request)\n");
    out.push_str("    if (response.error) {\n");
    out.push_str("      throw new HewApiError(response.error)\n");
    out.push_str("    }\n");
    out.push_str("    return response.result as TResult\n");
    out.push_str("  }\n\n");
    out.push_str("  // A plain request to a model-mutating command is wrapped by the\n");
    out.push_str("  // dispatcher into a one-command transaction (HEW_API.md section 6) and\n");
    out.push_str("  // answered with {results: [<own result>], label} -- unwrap it so\n");
    out.push_str("  // every method resolves with its own declared result type.\n");
    out.push_str(
        "  private async mutate<TResult>(method: string, params: unknown): Promise<TResult> {\n",
    );
    out.push_str(
        "    const envelope = await this.call<{ results: [TResult]; label: string }>(method, params)\n",
    );
    out.push_str("    return envelope.results[0]\n");
    out.push_str("  }\n\n");

    for (ns, cmds) in &namespaces {
        out.push_str(&format!("  readonly {ns} = {{\n"));
        for cmd in cmds {
            let (ns, name) = split_command(cmd.name);
            let stem = type_stem(ns, name);
            let method = camel_case(name);
            // hew.doc.transact IS the transaction: its declared result is
            // the envelope itself, so it must not be unwrapped.
            let via = match cmd.class {
                CommandClass::ModelMutating if cmd.name != "hew.doc.transact" => "mutate",
                _ => "call",
            };
            out.push_str(&format!(
                "    {method}: (params: {stem}Params): Promise<{stem}Result> => this.{via}('{}', params),\n",
                cmd.name
            ));
        }
        out.push_str("  }\n\n");
    }
    // Trim the blank line the last namespace block's trailing "\n\n" would
    // otherwise leave before the class's closing brace.
    while out.ends_with("\n\n") {
        out.pop();
    }
    out.push_str("}\n");

    out
}

// ------------------------------------------------- API reference

/// Generates `docs/API_REFERENCE.gen.md` — one section per namespace, one
/// entry per command: name, version, tier, class, served, summary, the
/// params/result JSON Schemas verbatim, and the refusal inventory with
/// explanations pulled from `api::refusal::explanation_for` wherever the
/// UI copy table (`app/src/kernelErrors.ts`, via `refusal_copy.gen.rs`)
/// has one.
pub fn generate_api_reference(registry: &Registry) -> String {
    let mut out = String::new();

    out.push_str("<!--\n");
    out.push_str("GENERATED from crates/api registry — do not edit; regenerate with:\n");
    out.push_str(&format!("  {REGEN_COMMAND}\n"));
    out.push_str("-->\n\n");
    out.push_str("# Hew API Reference\n\n");
    out.push_str(
        "This is the mechanically published form of the command registry in\n\
         `crates/api/src/registry.rs` (docs/HEW_API.md §9: \"published from\n\
         it\"). It is the per-command companion to HEW_API.md, which defines\n\
         the protocol every command obeys; this document lists what each one\n\
         actually is. One section per namespace, one entry per command, in\n\
         registry order.\n\n\
         New to the API? Read docs/API_GUIDE.md first — how to connect, what\n\
         a session looks like, and worked examples of the idioms these\n\
         entries assume (transactions, `$ref`, face locators, refusals).\n\n",
    );

    let mut current_ns: Option<&str> = None;
    for cmd in registry.commands() {
        let (ns, _) = split_command(cmd.name);
        if current_ns != Some(ns) {
            out.push_str(&format!("## hew.{ns}\n\n"));
            current_ns = Some(ns);
        }

        out.push_str(&format!("### `{}`\n\n", cmd.name));
        out.push_str(&format!(
            "- **Version:** {}\n- **Tier:** {}\n- **Class:** {}\n- **Served:** {}\n\n",
            cmd.version,
            tier_label(cmd.tier),
            class_label(cmd.class),
            served_label(cmd.served),
        ));
        out.push_str(&format!("{}\n\n", cmd.summary));
        if !cmd.implemented {
            out.push_str(
                "> **Not yet implemented.** Every call answers the `unimplemented` \
                 refusal (docs/HEW_API.md §14's burn-down posture).\n\n",
            );
        }

        out.push_str("**Params schema:**\n\n```json\n");
        out.push_str(&serde_json::to_string_pretty(&cmd.params_schema).expect("schema serializes"));
        out.push_str("\n```\n\n");

        out.push_str("**Result schema:**\n\n```json\n");
        out.push_str(&serde_json::to_string_pretty(&cmd.result_schema).expect("schema serializes"));
        out.push_str("\n```\n\n");

        if cmd.refusals.is_empty() {
            out.push_str("**Refusals:** none.\n\n");
        } else {
            out.push_str("**Refusals:**\n\n");
            for refusal in &cmd.refusals {
                match explanation_for(refusal) {
                    Some(explanation) => out.push_str(&format!("- `{refusal}` — {explanation}\n")),
                    None => out.push_str(&format!("- `{refusal}`\n")),
                }
            }
            out.push('\n');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_command_splits_the_three_dot_segments() {
        assert_eq!(
            split_command("hew.sketch.draw_rect"),
            ("sketch", "draw_rect")
        );
        assert_eq!(split_command("hew.meta.hello"), ("meta", "hello"));
    }

    #[test]
    fn camel_case_matches_the_spec_example() {
        assert_eq!(camel_case("draw_rect"), "drawRect");
        assert_eq!(camel_case("push_pull"), "pushPull");
        assert_eq!(camel_case("set_default"), "setDefault");
    }

    #[test]
    fn schema_to_ts_maps_primitives_enums_and_arrays() {
        assert_eq!(
            schema_to_ts(&serde_json::json!({"type": "string"})),
            "string"
        );
        assert_eq!(
            schema_to_ts(&serde_json::json!({"type": "integer"})),
            "number"
        );
        assert_eq!(
            schema_to_ts(&serde_json::json!({"type": "string", "enum": ["a", "b"]})),
            "\"a\" | \"b\""
        );
        assert_eq!(
            schema_to_ts(&serde_json::json!({
                "type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3
            })),
            "[number, number, number]"
        );
        assert_eq!(
            schema_to_ts(&serde_json::json!({"type": "array", "items": {"type": "string"}})),
            "string[]"
        );
    }

    #[test]
    fn schema_to_ts_falls_back_to_unspecified_shape_for_permissive_schemas() {
        assert_eq!(schema_to_ts(&serde_json::json!({})), "UnspecifiedShape");
        assert_eq!(
            schema_to_ts(&serde_json::json!({"type": "object"})),
            "UnspecifiedShape"
        );
        assert_eq!(
            schema_to_ts(&serde_json::json!({"type": "object", "description": "x"})),
            "UnspecifiedShape"
        );
    }

    #[test]
    fn schema_to_ts_renders_object_with_properties_inline() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": {"type": "string"}, "b": {"type": "number"} },
            "required": ["a"]
        });
        assert_eq!(schema_to_ts(&schema), "{ a: string; b?: number }");
    }

    #[test]
    fn generate_ts_sdk_produces_valid_looking_output_for_every_command() {
        let registry = Registry::protocol_1();
        let ts = generate_ts_sdk(&registry);
        assert!(ts.starts_with("// GENERATED from crates/api registry"));
        assert!(ts.contains("export interface HewTransport"));
        assert!(ts.contains("export class HewApiClient"));
        assert!(ts.contains("HewErrorCode"));
        assert!(ts.contains("drawRect: (params: SketchDrawRectParams)"));
        assert!(ts.contains("readonly sketch = {"));
        for cmd in registry.commands() {
            let (ns, name) = split_command(cmd.name);
            let stem = type_stem(ns, name);
            assert!(
                ts.contains(&format!("{stem}Params")),
                "missing {stem}Params for {}",
                cmd.name
            );
        }
    }

    #[test]
    fn generate_api_reference_covers_every_command_and_a_known_explanation() {
        let registry = Registry::protocol_1();
        let md = generate_api_reference(&registry);
        assert!(md.starts_with("<!--\nGENERATED from crates/api registry"));
        assert!(md.contains("## hew.sketch"));
        assert!(md.contains("### `hew.sketch.draw_rect`"));
        assert!(
            md.contains("`distance_too_small` — That distance is too small to build anything.")
        );
        for cmd in registry.commands() {
            assert!(
                md.contains(&format!("### `{}`", cmd.name)),
                "missing section for {}",
                cmd.name
            );
        }
    }
}
