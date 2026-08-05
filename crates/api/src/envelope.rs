//! The JSON-RPC 2.0 envelope (docs/HEW_API.md §4): requests, responses,
//! error objects, and the error-code inventory. These Rust types ARE the
//! API; JSON is their encoding at process boundaries (§3), so an
//! in-process caller pays no serialization at all.

use serde::{Deserialize, Serialize};

/// A JSON-RPC request id: number or string (JSON-RPC 2.0 §4). A request
/// with no id is a notification — client-originated notifications are
/// invalid at protocol 1 and dropped unexecuted (§4.1), so the dispatcher
/// only ever answers id-carrying requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    Text(String),
}

/// One request frame. `params` is always a single JSON object, never
/// positional (§4.1); absent params deserialize as `None` and are treated
/// as the empty object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// One response frame: exactly one of `result` / `error` is present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    /// Mirrors the request's id. `None` (serialized as JSON `null`) only
    /// for a response to a frame whose id was unreadable.
    pub id: Option<RequestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

/// A JSON-RPC error object. For refusals (`code == codes::REFUSED`),
/// `data` carries the canonical shape of §4.4: `refusal`, `failed_index`,
/// `failed_method`, `detail`, `explanation` — always all five, whether
/// the envelope was a transaction or a plain request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Response {
    /// A success frame.
    pub fn ok(id: Option<RequestId>, result: serde_json::Value) -> Response {
        Response {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// An error frame.
    pub fn err(id: Option<RequestId>, code: i64, message: &str) -> Response {
        Response {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(ErrorObject {
                code,
                message: message.to_string(),
                data: None,
            }),
        }
    }

    /// An error frame carrying structured `data`.
    pub fn err_with(
        id: Option<RequestId>,
        code: i64,
        message: &str,
        data: serde_json::Value,
    ) -> Response {
        Response {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(ErrorObject {
                code,
                message: message.to_string(),
                data: Some(data),
            }),
        }
    }
}

/// The protocol's error-code inventory (docs/HEW_API.md §4.4): the three
/// standard JSON-RPC codes the dispatcher can answer with, plus Hew's
/// reserved-range extensions. Pinned at protocol 1 — additive only.
pub mod codes {
    /// Malformed JSON reached the dispatcher (host transports usually
    /// catch this first).
    pub const PARSE_ERROR: i64 = -32700;
    /// A method the registry has never heard of — NEVER a real command
    /// the connection was simply not granted (that is [`NOT_PERMITTED`]).
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Malformed params: unknown fields, wrong types, missing required
    /// fields, static `$ref` defects, unbalanced context, a solitary
    /// command sharing an envelope (§6).
    pub const INVALID_PARAMS: i64 = -32602;
    /// The kernel declined a well-formed command; `data` carries the
    /// typed refusal, document untouched (§4.4).
    pub const REFUSED: i64 = -32000;
    /// A real registry command outside the connection's granted profile
    /// (§10) — carries the offending index for transactions.
    pub const NOT_PERMITTED: i64 = -32001;
    /// No document attached to this connection (§4.2).
    pub const NO_DOCUMENT: i64 = -32002;
    /// A kernel invariant failed and was rolled back — a bug to report,
    /// not to handle (§4.4).
    pub const INTERNAL_FAULT: i64 = -32003;
    /// No successful `hew.meta.hello` on this connection yet (§4.2).
    pub const NOT_READY: i64 = -32004;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_with_and_without_id() {
        let json =
            r#"{"jsonrpc":"2.0","id":4,"method":"hew.query.entity","params":{"id":"obj_5f3a"}}"#;
        let req: Request = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, Some(RequestId::Number(4)));
        assert_eq!(req.method, "hew.query.entity");

        let notification: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"hew.event.x"}"#).unwrap();
        assert_eq!(notification.id, None);
    }

    #[test]
    fn response_serializes_exactly_one_of_result_or_error() {
        let ok = Response::ok(Some(RequestId::Number(1)), serde_json::json!({"a": 1}));
        let v = serde_json::to_value(&ok).unwrap();
        assert!(v.get("result").is_some() && v.get("error").is_none());

        let err = Response::err(Some(RequestId::Text("x".into())), codes::REFUSED, "refused");
        let v = serde_json::to_value(&err).unwrap();
        assert!(v.get("error").is_some() && v.get("result").is_none());
    }
}
