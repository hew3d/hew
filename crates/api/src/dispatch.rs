//! The dispatcher: one pure function from request to response, per
//! connection (docs/HEW_API.md §1). The scaffold implements the
//! connection lifecycle — `hello` gating, profile enforcement, attachment
//! — and answers every not-yet-implemented command with the
//! `unimplemented` refusal, which is what makes the declared registry a
//! visible burn-down list (§14).

use crate::commands::{self, CmdError, Ctx, FaceTokens};
use crate::envelope::{Request, RequestId, Response, codes};
use crate::host::Host;
use crate::registry::{CommandClass, Profile, Registry};
use crate::transact::{self, TxError};

/// The protocol version this dispatcher speaks (docs/HEW_API.md §4.2).
pub const PROTOCOL_VERSION: u32 = 1;

/// What dispatch did with a frame.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// An id-carrying request: here is its response.
    Reply(Response),
    /// A client-originated notification (no `id`): dropped UNEXECUTED —
    /// a mutation that cannot answer would silently break the
    /// refusals-are-answers contract (§4.1).
    Dropped,
}

/// One client connection's dispatch state: the granted profile and the
/// handshake/attachment lifecycle. The host owns the `kernel::Document`
/// and lends it per dispatch — transports and document ownership are host
/// concerns (§3, rule 1).
#[derive(Debug)]
pub struct Connection {
    profile: Profile,
    /// The connection identity the host assigned (reported as history
    /// origin for this connection's transactions — §6.1).
    identity: String,
    hello_done: bool,
    attached: bool,
    registry: Registry,
}

impl Connection {
    /// A connection granted `profile`, identified (for history origin) as
    /// `identity`.
    pub fn new(profile: Profile, identity: &str) -> Connection {
        Connection {
            profile,
            identity: identity.to_string(),
            hello_done: false,
            attached: false,
            registry: Registry::protocol_1(),
        }
    }

    /// The registry this connection dispatches against.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The host-assigned connection identity (§6.1's origin).
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Dispatch one frame against the attached document.
    ///
    /// The scaffold's lifecycle contract, in evaluation order (§4):
    /// any frame without an id is dropped unexecuted; an unknown method is
    /// `-32601` (that code is reserved for methods the registry has never
    /// heard of, so it answers even pre-`hello`); before a successful
    /// `hello`, every known method but `hello` is `-32004`; a
    /// granted-profile miss is `-32001`; a command that operates on the
    /// attached document answers `-32002` while unattached — the
    /// attachment-establishing commands (`hew.doc.new`/`open`/`attach`)
    /// and the host-wide `hew.meta.*` surface are exempt (§4.2: new/open
    /// auto-attach; `documents` is what you call to FIND a document); and
    /// every declared, permitted, not-yet-implemented command answers the
    /// `unimplemented` refusal in the canonical §4.4 shape.
    pub fn dispatch(
        &mut self,
        doc: &mut kernel::Document,
        host: &mut dyn Host,
        request: Request,
    ) -> DispatchOutcome {
        let Some(id) = request.id.clone() else {
            return DispatchOutcome::Dropped;
        };
        DispatchOutcome::Reply(self.reply(doc, host, id, request))
    }

    fn reply(
        &mut self,
        doc: &mut kernel::Document,
        host: &mut dyn Host,
        id: RequestId,
        request: Request,
    ) -> Response {
        let id = Some(id);
        if request.jsonrpc != "2.0" {
            return Response::err(id, codes::INVALID_PARAMS, "jsonrpc must be \"2.0\"");
        }
        let Some(cmd) = self.registry.get(&request.method) else {
            return Response::err(id, codes::METHOD_NOT_FOUND, "unknown method");
        };
        if !self.hello_done {
            if cmd.name == "hew.meta.hello" {
                return self.hello(id, request.params);
            }
            return Response::err(id, codes::NOT_READY, "hew.meta.hello first");
        }
        if !self.profile.grants(cmd) {
            return Response::err(
                id,
                codes::NOT_PERMITTED,
                "method not in the granted profile",
            );
        }
        let class = cmd.class;
        let name = cmd.name;
        match name {
            "hew.meta.hello" => self.hello(id, request.params),
            "hew.meta.capabilities" => self.capabilities(id),
            "hew.doc.attach" => {
                // Scaffold posture: the single lent document is the
                // attachable one.
                self.attached = true;
                Response::ok(id, serde_json::json!({}))
            }
            "hew.doc.transact" => {
                let params = request.params.unwrap_or(serde_json::json!({}));
                // A one-command envelope holding a solitary command is that
                // command's canonical MCP invocation (§6.4, §13); the
                // attachment-establishing ones keep their exemption and
                // auto-attach on success, and `attach` itself is served
                // inline here (its state lives on the connection).
                let single_solitary = single_solitary_method(&self.registry, &params);
                if single_solitary.as_deref() == Some("hew.doc.attach") {
                    self.attached = true;
                    return Response::ok(
                        id,
                        serde_json::json!({ "results": [{}], "label": "hew.doc.attach" }),
                    );
                }
                let establishes = matches!(
                    single_solitary.as_deref(),
                    Some("hew.doc.new") | Some("hew.doc.open")
                );
                if !self.attached && !establishes {
                    return Response::err(id, codes::NO_DOCUMENT, "no document attached");
                }
                let response = self.tx_response(id, doc, host, &params);
                if establishes && response.error.is_none() {
                    self.attached = true;
                }
                response
            }
            _ => {
                // Attachment-establishing and host-wide commands need no
                // attached document (§4.2); everything else does. `new`
                // and `open` auto-attach on success.
                let establishes = matches!(name, "hew.doc.new" | "hew.doc.open");
                let host_wide = name == "hew.meta.documents";
                if !(self.attached || establishes || host_wide) {
                    return Response::err(id, codes::NO_DOCUMENT, "no document attached");
                }
                match class {
                    // A model-mutating plain request is exactly a
                    // one-command transaction (§6.1), label defaulted
                    // from the command.
                    CommandClass::ModelMutating => {
                        let tx = serde_json::json!({
                            "commands": [{
                                "method": name,
                                "params": request.params.unwrap_or(serde_json::json!({})),
                            }]
                        });
                        let response = self.tx_response(id, doc, host, &tx);
                        if establishes && response.error.is_none() {
                            self.attached = true;
                        }
                        response
                    }
                    // Read-only and solitary commands run bare: no
                    // bracket, no undo entry (§6.4).
                    CommandClass::ReadOnly | CommandClass::Solitary => {
                        let response = self.run_bare(
                            id,
                            doc,
                            host,
                            name,
                            &request.params.unwrap_or(serde_json::json!({})),
                        );
                        if establishes && response.error.is_none() {
                            self.attached = true;
                        }
                        response
                    }
                }
            }
        }
    }

    /// Executes a transaction envelope and shapes its response.
    fn tx_response(
        &mut self,
        id: Option<RequestId>,
        doc: &mut kernel::Document,
        host: &mut dyn Host,
        params: &serde_json::Value,
    ) -> Response {
        match transact::run(
            doc,
            host,
            &self.registry,
            self.profile,
            &self.identity,
            params,
        ) {
            Ok(result) => Response::ok(id, result),
            Err(TxError::Invalid(msg)) => Response::err(id, codes::INVALID_PARAMS, &msg),
            Err(TxError::NotPermitted(index)) => Response::err_with(
                id,
                codes::NOT_PERMITTED,
                "method not in the granted profile",
                serde_json::json!({ "index": index }),
            ),
            Err(TxError::Refused {
                index,
                method,
                refusal,
            }) => Response::err_with(
                id,
                codes::REFUSED,
                "refused",
                refusal.into_data(index, &method),
            ),
            Err(TxError::Internal(msg)) => Response::err(id, codes::INTERNAL_FAULT, &msg),
        }
    }

    /// Runs a read-only or solitary command directly — no bracket.
    fn run_bare(
        &mut self,
        id: Option<RequestId>,
        doc: &mut kernel::Document,
        host: &mut dyn Host,
        name: &str,
        params: &serde_json::Value,
    ) -> Response {
        let Some(handler) = commands::handler(name) else {
            return Response::err_with(
                id,
                codes::REFUSED,
                "refused",
                crate::refusal::Refusal::api(
                    "unimplemented",
                    &format!("{name} is declared but not implemented yet in this build."),
                )
                .into_data(0, name),
            );
        };
        let mut face_tokens = FaceTokens::new();
        let mut ctx = Ctx {
            doc,
            host,
            face_tokens: &mut face_tokens,
            current_label: None,
        };
        match handler(&mut ctx, params) {
            Ok(result) => Response::ok(id, result),
            Err(CmdError::Params(msg)) => Response::err(id, codes::INVALID_PARAMS, &msg),
            Err(CmdError::Refusal(refusal)) => {
                Response::err_with(id, codes::REFUSED, "refused", refusal.into_data(0, name))
            }
            Err(CmdError::Internal(msg)) => Response::err(id, codes::INTERNAL_FAULT, &msg),
        }
    }

    fn hello(&mut self, id: Option<RequestId>, params: Option<serde_json::Value>) -> Response {
        let requested = params
            .as_ref()
            .and_then(|p| p.get("protocol"))
            .and_then(serde_json::Value::as_u64);
        match requested {
            Some(v) if v == u64::from(PROTOCOL_VERSION) => {}
            _ => {
                return Response::err_with(
                    id,
                    codes::INVALID_PARAMS,
                    "unsupported protocol version",
                    serde_json::json!({ "speaks": [PROTOCOL_VERSION] }),
                );
            }
        }
        self.hello_done = true;
        Response::ok(
            id,
            serde_json::json!({
                "protocol": PROTOCOL_VERSION,
                "app": { "name": "hew", "version": env!("CARGO_PKG_VERSION") },
                "profile": match self.profile {
                    Profile::Core => "core",
                    Profile::App => "app",
                },
                "encoding": "json",
                "documents": [],
            }),
        )
    }

    fn capabilities(&self, id: Option<RequestId>) -> Response {
        let commands: Vec<serde_json::Value> = self
            .registry
            .commands()
            .filter(|c| self.profile.grants(c))
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "version": c.version,
                    "summary": c.summary,
                    "class": match c.class {
                        CommandClass::ModelMutating => "model_mutating",
                        CommandClass::ReadOnly => "read_only",
                        CommandClass::Solitary => "solitary",
                    },
                    "params": c.params_schema,
                    "result": c.result_schema,
                    "refusals": c.refusals,
                    "implemented": c.implemented,
                })
            })
            .collect();
        Response::ok(id, serde_json::json!({ "commands": commands }))
    }
}

/// The method name of a one-command transact envelope whose single
/// command is solitary-class — `None` for every other envelope shape.
fn single_solitary_method(registry: &Registry, params: &serde_json::Value) -> Option<String> {
    let commands = params.get("commands")?.as_array()?;
    if commands.len() != 1 {
        return None;
    }
    let method = commands[0].get("method")?.as_str()?;
    let decl = registry.get(method)?;
    (decl.class == CommandClass::Solitary).then(|| method.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: i64, method: &str, params: serde_json::Value) -> Request {
        Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(id)),
            method: method.to_string(),
            params: Some(params),
        }
    }

    fn hello(conn: &mut Connection, doc: &mut kernel::Document) {
        let out = conn.dispatch(
            doc,
            &mut crate::host::NoHost,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        let DispatchOutcome::Reply(r) = out else {
            panic!("hello replies")
        };
        assert!(r.error.is_none(), "hello succeeds: {:?}", r.error);
    }

    #[test]
    fn everything_before_hello_is_not_ready() {
        let mut conn = Connection::new(Profile::Core, "test");
        let mut doc = kernel::Document::new();
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &mut doc,
            &mut crate::host::NoHost,
            req(1, "hew.query.scene", serde_json::json!({})),
        ) else {
            panic!()
        };
        assert_eq!(r.error.unwrap().code, codes::NOT_READY);
    }

    #[test]
    fn unknown_method_is_method_not_found_even_before_hello() {
        let mut conn = Connection::new(Profile::Core, "test");
        let mut doc = kernel::Document::new();
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &mut doc,
            &mut crate::host::NoHost,
            req(1, "hew.nope.nothing", serde_json::json!({})),
        ) else {
            panic!()
        };
        assert_eq!(r.error.unwrap().code, codes::METHOD_NOT_FOUND);
    }

    /// `hew.view.snapshot` used to be `app`-only; core now grants it
    /// specifically (docs/design/headless-snapshot.md — it has a headless
    /// render path), so dispatching it under `Core` reaches the host
    /// instead of being turned away at the profile gate. `NoHost` then
    /// answers its own typed refusal, not `NOT_PERMITTED`. No other
    /// `hew.view.*` command exists yet to demonstrate a live
    /// `NOT_PERMITTED` example through the dispatcher — that boundary is
    /// pinned directly at the registry level instead
    /// (`registry.rs`'s `core_profile_grants_snapshot_but_withholds_other_view_commands`).
    #[test]
    fn core_now_reaches_the_host_for_snapshot_instead_of_not_permitted() {
        let mut conn = Connection::new(Profile::Core, "test");
        let mut doc = kernel::Document::new();
        hello(&mut conn, &mut doc);
        conn.dispatch(
            &mut doc,
            &mut crate::host::NoHost,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &mut doc,
            &mut crate::host::NoHost,
            req(2, "hew.view.snapshot", serde_json::json!({})),
        ) else {
            panic!()
        };
        let err = r.error.unwrap();
        assert_eq!(
            err.code,
            codes::REFUSED,
            "reaches the host, not the profile gate"
        );
        assert_eq!(
            err.data.unwrap()["refusal"],
            "host_capability_missing",
            "NoHost's own refusal, not NOT_PERMITTED"
        );
    }

    #[test]
    fn unimplemented_commands_refuse_in_the_canonical_shape() {
        let mut conn = Connection::new(Profile::Core, "test");
        let mut doc = kernel::Document::new();
        hello(&mut conn, &mut doc);
        conn.dispatch(
            &mut doc,
            &mut crate::host::NoHost,
            req(2, "hew.doc.attach", serde_json::json!({})),
        );
        // Every REAL protocol-1 command now has a `commands::handler`
        // entry (§14's burn-down is empty — `hew.guide.angular` was the
        // last one, closed out by composing `Document::add_guide_line`
        // client-side; see docs/design/api-kernel-map.md §1.11), so
        // there is no longer a declared-but-unimplemented command to
        // dispatch by name. `run_bare` is what actually produces the
        // "unimplemented" shape (`reply` only reaches it once the
        // registry lookup, hello-gate, and profile grant have already
        // passed), and it does so purely from `commands::handler(name)`
        // being `None` — so calling it directly with a name that will
        // never be real exercises the exact same branch a stale
        // registry entry would have, without inventing protocol surface
        // just to keep the fixture alive.
        let r = conn.run_bare(
            Some(RequestId::Number(3)),
            &mut doc,
            &mut crate::host::NoHost,
            "hew.test.not_a_real_command",
            &serde_json::json!({}),
        );
        let err = r.error.unwrap();
        assert_eq!(err.code, codes::REFUSED);
        let data = err.data.unwrap();
        for key in [
            "refusal",
            "failed_index",
            "failed_method",
            "detail",
            "explanation",
        ] {
            assert!(
                data.get(key).is_some(),
                "canonical refusal shape lacks {key}"
            );
        }
        assert_eq!(data["refusal"], "unimplemented");
        assert_eq!(data["failed_method"], "hew.test.not_a_real_command");
    }

    #[test]
    fn notifications_are_dropped_unexecuted() {
        let mut conn = Connection::new(Profile::Core, "test");
        let mut doc = kernel::Document::new();
        hello(&mut conn, &mut doc);
        let out = conn.dispatch(
            &mut doc,
            &mut crate::host::NoHost,
            Request {
                jsonrpc: "2.0".to_string(),
                id: None,
                method: "hew.solid.extrude".to_string(),
                params: None,
            },
        );
        assert!(matches!(out, DispatchOutcome::Dropped));
    }

    #[test]
    fn hello_negotiates_protocol_and_capabilities_reflect_the_profile() {
        let mut conn = Connection::new(Profile::Core, "test");
        let mut doc = kernel::Document::new();
        // Wrong protocol refused with the versions we speak.
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &mut doc,
            &mut crate::host::NoHost,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 99})),
        ) else {
            panic!()
        };
        assert_eq!(r.error.as_ref().unwrap().code, codes::INVALID_PARAMS);

        hello(&mut conn, &mut doc);
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &mut doc,
            &mut crate::host::NoHost,
            req(1, "hew.meta.capabilities", serde_json::json!({})),
        ) else {
            panic!()
        };
        let result = r.result.unwrap();
        let commands = result["commands"].as_array().unwrap();
        assert!(commands.len() >= 50);
        // Core grants hew.view.snapshot specifically (headless render
        // path — docs/design/headless-snapshot.md): capabilities lists
        // it, unlike a future app-only hew.view.* addition.
        assert!(
            commands
                .iter()
                .any(|c| c["name"].as_str().unwrap() == "hew.view.snapshot"),
            "core capabilities include the headless-capable snapshot command"
        );
    }
}
