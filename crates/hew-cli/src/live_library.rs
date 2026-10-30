//! `--live` handling for `hew.library.*` (docs/agents/HEW_API.md §12, §13;
//! docs/design/v1.1-cycle.md's Lane B).
//!
//! `crates/wasm-api`'s `LiveHost` (the desktop app's in-process host when
//! it serves a `--live` connection) has no filesystem — the library folder
//! lives on whichever machine `hew-cli` itself runs on, which is where the
//! `HEW_LIBRARY_DIR`/`library.json` resolution actually happens. So every
//! `hew.library.*` envelope is pre-resolved HERE, on the client side that
//! does have a filesystem, before anything reaches the remote app:
//!
//! - `list`/`describe`/`remove`/`update_meta` never touch the live
//!   document at all — [`dispatch_local`] answers them ENTIRELY on this
//!   side, through the exact `api::commands::library` handlers headless
//!   dispatch uses (same code, same [`CliHost`]), never touching the
//!   socket.
//! - `insert {item}` DOES need the remote (only it can mutate the live
//!   document) — [`resolve_insert_item`] rewrites `item` to
//!   `bytes_base64`/`content_hash` first, so the remote's `insert` command
//!   never has to ask ITS host (which has none) to read anything.
//! - `save` also needs the remote (only it can `extract_item` from the
//!   live document) — [`force_return_bytes`] forces `return_bytes: true`
//!   on the way out; [`finish_save`] takes the returned bytes, writes them
//!   to THIS side's library folder, and rewrites the result to carry the
//!   real local `path`.
//!
//! Every entry point here handles both the bare command shape and the
//! one-command `hew.doc.transact` wrapping it (§6.4's canonical MCP
//! invocation for anything that is not read-only) — mirrors
//! `crate::run::take_client_write_path`'s same two-shape handling for
//! `hew.doc.save`/`export`, for the identical reason: an MCP client has no
//! other shape to send a mutating command in.

use crate::host::CliHost;
use api::commands::{self, CmdError, Ctx, FaceTokens};
use api::envelope::codes;
use api::{Host, Refusal, RequestId, Response};
use serde_json::Value;

/// `hew.library.*` commands answered ENTIRELY on this side in `--live`
/// mode — they never touch the document, so there is nothing the remote
/// app could add.
const LOCAL_ONLY: &[&str] = &[
    "hew.library.list",
    "hew.library.describe",
    "hew.library.remove",
    "hew.library.update_meta",
];

/// The bare method name, or — for a one-command `hew.doc.transact`
/// envelope — the single command's method: the same "is this envelope
/// actually just X" test `take_client_write_path` uses.
fn single_command_method(method: &str, params: &Value) -> Option<String> {
    if method != "hew.doc.transact" {
        return Some(method.to_string());
    }
    let commands = params.get("commands")?.as_array()?;
    let [only] = commands.as_slice() else {
        return None;
    };
    only.get("method")?.as_str().map(str::to_string)
}

/// Whichever `hew.library.*` method `method`/`params` names, bare or
/// transact-wrapped — `None` for anything else.
pub(crate) fn library_method(method: &str, params: &Value) -> Option<String> {
    single_command_method(method, params).filter(|m| m.starts_with("hew.library."))
}

/// Runs a params transform over a bare command's params, or over the
/// single wrapped command's params inside a one-command
/// `hew.doc.transact` envelope — whichever shape `method`/`params` is.
fn map_command_params(
    method: &str,
    params: Value,
    f: impl FnOnce(Value) -> Result<Value, Box<Response>>,
) -> Result<Value, Box<Response>> {
    if method != "hew.doc.transact" {
        return f(params);
    }
    let mut params = params;
    let Some(inner) = params
        .get_mut("commands")
        .and_then(|c| c.as_array_mut())
        .and_then(|c| c.get_mut(0))
        .and_then(|c| c.get_mut("params"))
    else {
        return Ok(params);
    };
    let taken = inner.take();
    *inner = f(taken)?;
    Ok(params)
}

/// True for a method/envelope this module answers entirely on this side
/// (never touching the remote).
pub(crate) fn is_local_only(method: &str, params: &Value) -> bool {
    library_method(method, params).is_some_and(|m| LOCAL_ONLY.contains(&m.as_str()))
}

/// Runs a local-only `hew.library.*` command (bare OR transact-wrapped —
/// see [`is_local_only`]) through the exact handler headless dispatch
/// uses, and shapes the result exactly as the embedded dispatcher would:
/// a bare command's own result, or a one-command transact's
/// `{results: [...], label}` wrapper.
pub(crate) fn dispatch_local(method: &str, params: &Value) -> Response {
    let inner_method =
        library_method(method, params).expect("caller already checked is_local_only");
    let inner_params = if method == "hew.doc.transact" {
        params
            .pointer("/commands/0/params")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()))
    } else {
        params.clone()
    };
    let handler = commands::library::handler(&inner_method).expect("checked against LOCAL_ONLY");
    let mut doc = kernel::Document::new();
    let mut host = CliHost::new();
    let mut face_tokens = FaceTokens::new();
    // hew-cli is a first-party client: the embedded dispatcher's grant.
    let grant = api::Grant::core();
    let mut ctx = Ctx {
        doc: &mut doc,
        host: &mut host,
        face_tokens: &mut face_tokens,
        current_label: None,
        grant: &grant,
    };
    let id = Some(RequestId::Text("live-library".to_string()));
    match handler(&mut ctx, &inner_params) {
        Ok(result) => {
            let result = if method == "hew.doc.transact" {
                serde_json::json!({ "results": [result], "label": inner_method })
            } else {
                result
            };
            Response::ok(id, result)
        }
        Err(CmdError::Params(msg)) => Response::err(id, codes::INVALID_PARAMS, &msg),
        Err(CmdError::Refusal(refusal)) => Response::err_with(
            id,
            codes::REFUSED,
            "refused",
            refusal.into_data(0, &inner_method),
        ),
        Err(CmdError::Internal(msg)) => Response::err(id, codes::INTERNAL_FAULT, &msg),
    }
}

fn refusal_response(refusal: Refusal, method: &str) -> Response {
    Response::err_with(
        Some(RequestId::Text("live-library".to_string())),
        codes::REFUSED,
        "refused",
        refusal.into_data(0, method),
    )
}

fn unknown_item_response(item: &str, method: &str) -> Response {
    refusal_response(
        Refusal::api(
            "unknown_library_item",
            &format!("'{item}' does not name an item in the library (by id or path)."),
        )
        .with_detail(serde_json::json!({ "item": item })),
        method,
    )
}

/// Pre-resolves `hew.library.insert`'s `item` into `bytes_base64` +
/// `content_hash` (bare or transact-wrapped — see [`map_command_params`]),
/// so the remote never has to read anything through its own
/// (filesystem-less) host. Params that already carry `bytes_base64`, or a
/// method that isn't `hew.library.insert`, pass through unchanged.
pub(crate) fn resolve_insert_item(method: &str, params: Value) -> Result<Value, Box<Response>> {
    if library_method(method, &params).as_deref() != Some("hew.library.insert") {
        return Ok(params);
    }
    map_command_params(method, params, |inner| {
        let Some(obj) = inner.as_object() else {
            return Ok(inner);
        };
        if obj.contains_key("bytes_base64") {
            return Ok(inner);
        }
        let Some(item) = obj.get("item").and_then(|v| v.as_str()).map(str::to_string) else {
            // Neither item nor bytes_base64: let the remote answer the
            // "exactly one of" params refusal — no local work to do.
            return Ok(inner);
        };
        let host = CliHost::new();
        let listing = host
            .library_list()
            .map_err(|r| Box::new(refusal_response(r, "hew.library.insert")))?;
        let Some(entry) = listing
            .items
            .iter()
            .find(|e| e.path == item || e.id.as_deref() == Some(item.as_str()))
        else {
            return Err(Box::new(unknown_item_response(&item, "hew.library.insert")));
        };
        let read = host
            .library_read(&entry.path)
            .map_err(|r| Box::new(refusal_response(r, "hew.library.insert")))?;
        let mut inner = inner;
        let obj = inner.as_object_mut().expect("checked above");
        obj.remove("item");
        obj.insert(
            "bytes_base64".to_string(),
            Value::String(encode_base64(&read.bytes)),
        );
        obj.insert("content_hash".to_string(), Value::String(read.content_hash));
        Ok(inner)
    })
}

/// Forces `return_bytes: true` on a `hew.library.save` envelope (bare or
/// transact-wrapped): the remote's `hew.library.save` handler already
/// hands bytes back on request (`crates/api/src/commands/library.rs`),
/// which is exactly the shape [`finish_save`] needs — no NEW behavior on
/// the remote, just always asking for what it already offers. A method
/// that isn't `hew.library.save` passes through unchanged.
pub(crate) fn force_return_bytes(method: &str, params: Value) -> Value {
    if library_method(method, &params).as_deref() != Some("hew.library.save") {
        return params;
    }
    map_command_params(method, params, |inner| {
        let mut inner = inner;
        if let Some(obj) = inner.as_object_mut() {
            obj.insert("return_bytes".to_string(), Value::Bool(true));
        }
        Ok(inner)
    })
    .unwrap_or_else(|_| unreachable!("force_return_bytes's closure never errors"))
}

/// Whether `method`/`params` is a `hew.library.save` that names no
/// `source_doc` — the one case where a live client can fill in what the
/// remote host cannot know (the file the attached document came from).
pub(crate) fn lacks_source_doc(method: &str, params: &Value) -> bool {
    if library_method(method, params).as_deref() != Some("hew.library.save") {
        return false;
    }
    let inner = if method == "hew.doc.transact" {
        params.pointer("/commands/0/params")
    } else {
        Some(params)
    };
    inner.is_none_or(|p| p.get("source_doc").is_none())
}

/// Sets `source_doc` on a `hew.library.save` (bare or one-command
/// transaction), leaving everything else untouched.
pub(crate) fn with_source_doc(method: &str, params: Value, source_doc: &str) -> Value {
    map_command_params(method, params, |mut inner| {
        if let Some(obj) = inner.as_object_mut() {
            obj.insert(
                "source_doc".to_string(),
                Value::String(source_doc.to_string()),
            );
        }
        Ok(inner)
    })
    .unwrap_or_else(|_| unreachable!("with_source_doc's closure never errors"))
}

/// After a successful remote `hew.library.save` reply (bytes forced via
/// [`force_return_bytes`]), writes the item to THIS side's library folder
/// and returns the rewritten result (`{path, id}`, `bytes_base64` kept
/// only when `client_wanted_bytes` was true) — the user's library lives
/// wherever `hew-cli` itself resolves it, not necessarily the same
/// machine/profile the remote app's own (irrelevant, since it has no
/// filesystem) library folder would have named. `response` is the WHOLE
/// JSON-RPC response value; `None` (not a `save` reply, or an error reply)
/// means "nothing to do here, forward as-is".
pub(crate) fn finish_save(
    method: &str,
    original_params: &Value,
    response: &Value,
) -> Option<Result<Value, String>> {
    if library_method(method, original_params).as_deref() != Some("hew.library.save") {
        return None;
    }
    if response.get("error").is_some() {
        return None;
    }
    let client_wanted_bytes = {
        let inner = if method == "hew.doc.transact" {
            original_params.pointer("/commands/0/params")
        } else {
            Some(original_params)
        };
        inner
            .and_then(|p| p.get("return_bytes"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let b64 = response
        .pointer("/result/results/0/bytes_base64")
        .or_else(|| response.pointer("/result/bytes_base64"))
        .and_then(Value::as_str);
    let Some(b64) = b64 else {
        return Some(Err("the remote returned no item bytes to write".to_string()));
    };
    Some(write_saved_item(b64, client_wanted_bytes))
}

fn write_saved_item(b64: &str, keep_bytes: bool) -> Result<Value, String> {
    let bytes = crate::run::decode_base64(b64)
        .ok_or_else(|| "the saved item's bytes were malformed".to_string())?;
    let summary = kernel::read_item_summary(&bytes).map_err(|e| format!("{e:?}"))?;
    let raw_meta = summary
        .doc_attrs
        .get("hew.library")
        .cloned()
        .unwrap_or(Value::Null);
    let meta = library::parse_item_meta(&raw_meta);
    let category = meta.category.unwrap_or(library::Category::Model);
    let name = meta.name.unwrap_or_else(|| "item".to_string());
    let id = meta
        .id
        .ok_or_else(|| "the saved item carries no hew.library id".to_string())?;
    let dir = library::LibraryDir::resolve(None);
    let path = library::item_file_name(&name, &id, category);
    library::write(&dir, &path, &bytes).map_err(|e| format!("{path}: {e}"))?;
    let mut result = serde_json::json!({ "path": path, "id": id });
    if keep_bytes {
        result
            .as_object_mut()
            .expect("built as an object above")
            .insert("bytes_base64".to_string(), Value::String(b64.to_string()));
    }
    Ok(result)
}

/// RFC 4648 §4 standard-alphabet base64, WITH padding — the encoder half
/// of `crate::run::decode_base64`; both are small hand-rolled copies of
/// what `crates/api/src/commands/doc.rs` already carries (that copy is
/// `pub(super)`, private to `crates/api`), matching this crate's existing
/// no-new-dependency posture for base64.
fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied();
        let b2 = chunk.get(2).copied();
        let n =
            (u32::from(b0) << 16) | (u32::from(b1.unwrap_or(0)) << 8) | u32::from(b2.unwrap_or(0));
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(if b1.is_some() {
            ALPHABET[((n >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if b2.is_some() {
            ALPHABET[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn library_method_recognizes_bare_and_transact_wrapped_forms() {
        assert_eq!(
            library_method("hew.library.list", &json!({})).as_deref(),
            Some("hew.library.list")
        );
        assert_eq!(
            library_method(
                "hew.doc.transact",
                &json!({"commands":[{"method":"hew.library.save","params":{}}]})
            )
            .as_deref(),
            Some("hew.library.save")
        );
        assert_eq!(library_method("hew.solid.extrude", &json!({})), None);
        assert_eq!(
            library_method(
                "hew.doc.transact",
                &json!({"commands":[{"method":"hew.solid.extrude","params":{}}]})
            ),
            None
        );
        // Multi-command transactions are never treated as a single
        // library command, bare-name matching or not.
        assert_eq!(
            library_method(
                "hew.doc.transact",
                &json!({"commands":[
                    {"method":"hew.library.save","params":{}},
                    {"method":"hew.query.scene","params":{}}
                ]})
            ),
            None
        );
    }

    #[test]
    fn is_local_only_covers_exactly_the_four_read_side_commands() {
        for m in [
            "hew.library.list",
            "hew.library.describe",
            "hew.library.remove",
            "hew.library.update_meta",
        ] {
            assert!(is_local_only(m, &json!({})), "{m}");
        }
        for m in [
            "hew.library.insert",
            "hew.library.save",
            "hew.solid.extrude",
        ] {
            assert!(!is_local_only(m, &json!({})), "{m}");
        }
    }

    #[test]
    fn source_doc_is_added_only_to_a_library_save_that_lacks_one() {
        assert!(lacks_source_doc("hew.library.save", &json!({"name": "X"})));
        assert!(!lacks_source_doc(
            "hew.library.save",
            &json!({"name": "X", "source_doc": "/m/a.hew"})
        ));
        assert!(!lacks_source_doc("hew.library.list", &json!({})));
        let wrapped = json!({"commands":[{"method":"hew.library.save","params":{"name":"X"}}]});
        assert!(lacks_source_doc("hew.doc.transact", &wrapped));
        let out = with_source_doc("hew.doc.transact", wrapped, "/m/a.hew");
        assert_eq!(out["commands"][0]["params"]["source_doc"], "/m/a.hew");
        let bare = with_source_doc("hew.library.save", json!({"name": "X"}), "/m/a.hew");
        assert_eq!(bare["source_doc"], "/m/a.hew");
    }

    #[test]
    fn force_return_bytes_sets_the_flag_bare_and_wrapped() {
        let out = force_return_bytes("hew.library.save", json!({"name": "X"}));
        assert_eq!(out["return_bytes"], json!(true));

        let out = force_return_bytes(
            "hew.doc.transact",
            json!({"commands":[{"method":"hew.library.save","params":{"name":"X"}}]}),
        );
        assert_eq!(out["commands"][0]["params"]["return_bytes"], json!(true));

        // Untouched for anything else.
        let untouched = json!({"commands":[{"method":"hew.solid.extrude","params":{}}]});
        assert_eq!(
            force_return_bytes("hew.doc.transact", untouched.clone()),
            untouched
        );
    }

    #[test]
    fn resolve_insert_item_passes_through_bytes_base64_and_non_insert_methods() {
        let already = json!({"bytes_base64": "Zm9v"});
        assert_eq!(
            resolve_insert_item("hew.library.insert", already.clone()).unwrap(),
            already
        );
        let other = json!({"path": "/tmp/x"});
        assert_eq!(
            resolve_insert_item("hew.doc.save", other.clone()).unwrap(),
            other
        );
    }

    #[test]
    fn base64_round_trips() {
        for bytes in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            let encoded = encode_base64(bytes);
            assert_eq!(crate::run::decode_base64(&encoded).as_deref(), Some(bytes));
        }
    }
}
