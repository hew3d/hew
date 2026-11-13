//! The dispatcher: one pure function from request to response, per
//! connection (docs/agents/HEW_API.md §1). The scaffold implements the
//! connection lifecycle — `hello` gating, grant enforcement, attachment
//! — and answers every not-yet-implemented command with the
//! `unimplemented` refusal, which is what makes the declared registry a
//! visible burn-down list (§14).

use std::collections::{BTreeMap, BTreeSet};

use crate::commands::{self, CmdError, Ctx, FaceTokens};
use crate::envelope::{Request, RequestId, Response, codes};
use crate::grant::Grant;
use crate::host::Host;
use crate::refusal::Refusal;
use crate::registry::{CommandClass, CommandDecl, Provider, Registry};
use crate::transact::{self, TxError};

/// The protocol version this dispatcher speaks (docs/agents/HEW_API.md §4.2).
pub const PROTOCOL_VERSION: u32 = 1;

/// The token the next forward gets, from one counter for the whole
/// process: a token names its forward without the connection it was issued
/// on, which is what lets a host stamp a nested envelope with the forward
/// it serves (`Connection::dispatch_in_forward`) and a host compound match
/// that stamp unambiguously.
static NEXT_FORWARD_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What dispatch did with a frame.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// An id-carrying request: here is its response.
    Reply(Response),
    /// A client-originated notification (no `id`): dropped UNEXECUTED —
    /// a mutation that cannot answer would silently break the
    /// refusals-are-answers contract (§4.1).
    Dropped,
    /// A plugin-provided command, validated and ready for its plugin: the
    /// host routes it and hands the plugin's reply to
    /// [`Connection::complete_forward`], which shapes the response
    /// (docs/agents/HEW_API.md §10.2).
    Forward(ForwardRequest),
    /// The answer to a dry run (`hew.doc.transact` with `dry_run: true`,
    /// docs/agents/HEW_API.md §6.6), success and failure alike, a refusal
    /// made before the rehearsal ran (`compute_busy`, a permission or
    /// document check) included: a reply like
    /// [`DispatchOutcome::Reply`], after which the host does NOTHING it does
    /// for a mutation (no recording, no resync, no selection or view
    /// write-back): the document it holds was never touched.
    DryRun(Response),
}

impl DispatchOutcome {
    /// The response a reply or a dry run carries; `None` for a dropped
    /// notification or a forward.
    pub fn into_response(self) -> Option<Response> {
        match self {
            DispatchOutcome::Reply(r) | DispatchOutcome::DryRun(r) => Some(r),
            DispatchOutcome::Dropped | DispatchOutcome::Forward(_) => None,
        }
    }
}

/// The one-computation-at-a-time gate of a connection (`hew.compute.*`,
/// docs/agents/HEW_API.md §7): while it is held, a compute call on the same
/// connection answers `compute_busy` before any work. Every transport today
/// serves a connection's frames one at a time, so a serial caller never
/// sees it; a host that runs a computation off its dispatch thread holds
/// the gate ([`Connection::compute_gate`]) across it.
#[derive(Debug, Clone, Default)]
pub struct ComputeGate(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl ComputeGate {
    /// Takes the gate, or `None` when a computation already holds it. The
    /// guard releases it when dropped, on every exit path.
    pub fn try_acquire(&self) -> Option<ComputeGuard> {
        use std::sync::atomic::Ordering;
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| ComputeGuard(self.0.clone()))
    }

    /// Whether a computation holds the gate now.
    pub fn is_held(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// A held [`ComputeGate`]; dropping it releases the gate.
#[derive(Debug)]
pub struct ComputeGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for ComputeGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// `compute_busy`: a computation from this connection is still running.
fn compute_busy(id: Option<RequestId>, index: usize, method: &str) -> Response {
    Response::err_with(
        id,
        codes::REFUSED,
        "refused",
        Refusal::api(
            "compute_busy",
            "Another computation from this connection is still running. Wait for it, or drop the older request, then send this one.",
        )
        .with_detail(serde_json::json!({ "limit": 1 }))
        .into_data(index, method),
    )
}

/// Whether a request is a dry run: `hew.doc.transact` with `dry_run: true`
/// (docs/agents/HEW_API.md §6.6).
pub fn is_dry_run(method: &str, params: Option<&serde_json::Value>) -> bool {
    method == "hew.doc.transact"
        && params.and_then(|p| p.get("dry_run")) == Some(&serde_json::Value::Bool(true))
}

/// The compute command a request runs, with its index in the envelope: a
/// `hew.compute.*` request itself, or the first one inside a transact
/// envelope.
fn compute_in(name: &str, params: Option<&serde_json::Value>) -> Option<(usize, String)> {
    if name.starts_with("hew.compute.") {
        return Some((0, name.to_string()));
    }
    if name != "hew.doc.transact" {
        return None;
    }
    params?
        .get("commands")?
        .as_array()?
        .iter()
        .enumerate()
        .find_map(|(i, c)| {
            let m = c.get("method")?.as_str()?;
            m.starts_with("hew.compute.").then(|| (i, m.to_string()))
        })
}

/// A plugin command call the dispatcher validated and did not execute.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ForwardRequest {
    /// Hand this back to [`Connection::complete_forward`].
    pub token: u64,
    pub plugin: String,
    /// The command id within the plugin.
    pub local: String,
    /// The full registry name.
    pub method: String,
    pub request_id: RequestId,
    /// Validated against the declared params schema.
    pub params: serde_json::Value,
    pub mutating: bool,
    pub label: Option<String>,
    pub timeout_ms: u32,
    pub concurrency: plugin_manifest::Concurrency,
    pub caller: ForwardCaller,
}

/// Who issued a forwarded call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ForwardCaller {
    pub identity: String,
    #[serde(default)]
    pub plugin: Option<String>,
    #[serde(default)]
    pub app: Option<String>,
    /// The forward this request was issued while serving, if any: the
    /// token the host passed to [`Connection::dispatch_in_forward`]. What
    /// lets the host keep a nested forward inside the compound of the one
    /// that caused it (docs/agents/PLUGINS.md "Compounds").
    #[serde(default)]
    pub forward: Option<u64>,
}

/// What a plugin answered a forward with. Wire form: `{"ok": value}`,
/// `{"refusal": {"name", "explanation", "detail"}}`, or `{"failed":
/// "timeout"}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardReply {
    Ok(serde_json::Value),
    Refusal {
        name: String,
        explanation: String,
        #[serde(default)]
        detail: serde_json::Value,
    },
    Failed(ForwardFailure),
}

/// Why a forward got no answer from the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardFailure {
    Timeout,
    Crashed,
    Unavailable,
    NoHandler,
    Cancelled,
}

/// A forward this connection issued and has not completed.
#[derive(Debug)]
struct PendingForward {
    request_id: RequestId,
    /// The full registry name.
    name: String,
    result_schema: serde_json::Value,
    plugin: String,
    /// Issued for a sole-command `hew.doc.transact`: the result is answered
    /// in the transaction shape, under this label.
    wrapped: Option<String>,
}

/// What `reply` produced: a response now, or a forward to complete later.
enum Replied {
    Response(Response),
    Forward(ForwardRequest),
    DryRun(Response),
}

impl From<Response> for Replied {
    fn from(response: Response) -> Replied {
        Replied::Response(response)
    }
}

/// Why `hew.doc.attach` did not bind.
enum AttachError {
    /// Malformed params: `-32602`.
    Params(String),
    /// The host declined: a typed refusal.
    Refused(crate::refusal::Refusal),
}

/// One client connection's dispatch state: its grant and the
/// handshake/attachment lifecycle. The host owns the `kernel::Document`
/// and lends it per dispatch — transports and document ownership are host
/// concerns (§3, rule 1).
#[derive(Debug)]
pub struct Connection {
    grant: Grant,
    /// The connection identity the host assigned (reported as history
    /// origin for this connection's transactions — §6.1).
    identity: String,
    hello_done: bool,
    attached: bool,
    pending_forwards: BTreeMap<u64, PendingForward>,
    /// While positive, every command that can change the document answers
    /// `read_only_forward`.
    read_scope_depth: u32,
    /// The forward the frame being dispatched was issued while serving
    /// ([`Connection::dispatch_in_forward`]); `None` outside one.
    active_forward: Option<u64>,
    /// The events this connection subscribed to (`hew.event.subscribe`),
    /// each one its grant could receive when it subscribed.
    subscriptions: BTreeSet<String>,
    /// One `hew.compute.*` call at a time ([`ComputeGate`]).
    compute_gate: ComputeGate,
}

impl Connection {
    /// A connection holding `grant`, identified (for history origin) as
    /// `identity`.
    pub fn new(grant: Grant, identity: &str) -> Connection {
        Connection {
            grant,
            identity: identity.to_string(),
            hello_done: false,
            attached: false,
            pending_forwards: BTreeMap::new(),
            read_scope_depth: 0,
            active_forward: None,
            subscriptions: BTreeSet::new(),
            compute_gate: ComputeGate::default(),
        }
    }

    /// This connection's compute gate: a host that serves a `hew.compute.*`
    /// call off its dispatch thread holds it across the computation, so a
    /// second call answers `compute_busy` (docs/agents/HEW_API.md §7).
    pub fn compute_gate(&self) -> ComputeGate {
        self.compute_gate.clone()
    }

    /// Whether a frame of `event` goes to this connection: it subscribed,
    /// and its grant may receive the event now (checked at emit as well
    /// as at subscribe, so a narrowed grant takes effect without a
    /// reconnect).
    pub fn wants(&self, event: &str) -> bool {
        self.subscriptions.contains(event) && self.grant.may_receive(event)
    }

    /// [`Connection::wants`] for an event about document `document`: a
    /// restricted connection without `documents.other` hears only about the
    /// document it is bound to, and one bound to none hears about none (as
    /// [`Grant::filter_documents`] shows it none).
    pub fn wants_for(&self, event: &str, document: &str) -> bool {
        self.wants(event)
            && (!self.grant.is_restricted()
                || self.grant.may_address_other_documents()
                || self.grant.document.as_deref() == Some(document))
    }

    /// The events this connection is subscribed to, in name order.
    pub fn subscriptions(&self) -> impl Iterator<Item = &str> {
        self.subscriptions.iter().map(String::as_str)
    }

    /// The forward the frame now being dispatched was issued while serving,
    /// if any ([`Connection::dispatch_in_forward`]).
    pub fn active_forward(&self) -> Option<u64> {
        self.active_forward
    }

    /// [`Connection::dispatch`] for a frame a plugin issued while serving
    /// forward `forward_token`: dispatched exactly the same way, with the
    /// token stamped on any forward it issues in turn
    /// ([`ForwardCaller::forward`]). The kernel needs no stamp: an envelope
    /// dispatched while a compound is open commits as a child of the
    /// innermost one by construction; the stamp is what lets the host
    /// decide which envelopes may run now.
    pub fn dispatch_in_forward(
        &mut self,
        registry: &Registry,
        doc: &mut kernel::Document,
        host: &mut dyn Host,
        request: Request,
        forward_token: u64,
    ) -> DispatchOutcome {
        let outer = self.active_forward.replace(forward_token);
        let outcome = self.dispatch(registry, doc, host, request);
        self.active_forward = outer;
        outcome
    }

    /// How many forwards this connection issued that are not completed.
    pub fn pending_forward_count(&self) -> usize {
        self.pending_forwards.len()
    }

    /// Opens a read scope: until the matching [`Connection::end_read_scope`],
    /// any command that can change the document answers `-32001`
    /// `read_only_forward`. A host opens one around a forwarded read-only
    /// command, for the calls the plugin makes while serving it.
    pub fn begin_read_scope(&mut self) {
        self.read_scope_depth += 1;
    }

    /// Closes the innermost read scope; a no-op with none open.
    pub fn end_read_scope(&mut self) {
        self.read_scope_depth = self.read_scope_depth.saturating_sub(1);
    }

    /// What this connection may do (§10).
    pub fn grant(&self) -> &Grant {
        &self.grant
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
    /// `hello`, every known method but `hello` is `-32004`; a restricted
    /// grant naming a `document` without `documents.other` is `-32001`
    /// `document_not_granted`; a grant miss is `-32001`; a restricted grant
    /// setting a path parameter is `-32001` `path_not_granted`; a command
    /// that operates on the
    /// attached document answers `-32002` while unattached — the
    /// attachment-establishing commands (`hew.doc.new`/`open`/`attach`)
    /// and the host-wide `hew.meta.*` surface are exempt (§4.2: new/open
    /// auto-attach; `documents` is what you call to FIND a document); and
    /// every declared, permitted, not-yet-implemented command answers the
    /// `unimplemented` refusal in the canonical §4.4 shape. A
    /// plugin-provided command, past those checks, is validated against
    /// its params schema and comes back as [`DispatchOutcome::Forward`].
    pub fn dispatch(
        &mut self,
        registry: &Registry,
        doc: &mut kernel::Document,
        host: &mut dyn Host,
        request: Request,
    ) -> DispatchOutcome {
        let Some(id) = request.id.clone() else {
            return DispatchOutcome::Dropped;
        };
        // Every answer to a dry run is tagged as one, a refusal made
        // before the rehearsal ran (`compute_busy`, a permission or
        // document check) included: none of them touched the document.
        let dry_run = is_dry_run(&request.method, request.params.as_ref());
        match self.reply(registry, doc, host, id, request) {
            Replied::Response(response) if dry_run => DispatchOutcome::DryRun(response),
            Replied::Response(response) => DispatchOutcome::Reply(response),
            Replied::Forward(forward) => DispatchOutcome::Forward(forward),
            Replied::DryRun(response) => DispatchOutcome::DryRun(response),
        }
    }

    fn reply(
        &mut self,
        registry: &Registry,
        doc: &mut kernel::Document,
        host: &mut dyn Host,
        id: RequestId,
        request: Request,
    ) -> Replied {
        let id = Some(id);
        if request.jsonrpc != "2.0" {
            return Response::err(id, codes::INVALID_PARAMS, "jsonrpc must be \"2.0\"").into();
        }
        let Some(cmd) = registry.get(&request.method) else {
            return Response::err(id, codes::METHOD_NOT_FOUND, "unknown method").into();
        };
        if !self.hello_done {
            if cmd.name == "hew.meta.hello" {
                return self.hello(id, request.params, doc, host).into();
            }
            return Response::err(id, codes::NOT_READY, "hew.meta.hello first").into();
        }
        // A compound abort cannot undo a document replacement, so these
        // wait while a plugin command holds the document, whoever asks
        // (docs/agents/PLUGINS.md "Compounds").
        if doc.compound_open() && transact::DOCUMENT_REPLACING.contains(&cmd.name.as_str()) {
            return Response::err_with(
                id,
                codes::REFUSED,
                "refused",
                transact::document_busy().into_data(0, &cmd.name),
            )
            .into();
        }
        // Document binding (§10): a restricted grant addresses only the
        // document it is bound to, which is the one this dispatcher has, so
        // naming any document is reaching for another one.
        if request.document.is_some()
            && self.grant.is_restricted()
            && !self.grant.may_address_other_documents()
        {
            return Response::err_with(
                id,
                codes::NOT_PERMITTED,
                "document not granted to this connection",
                serde_json::json!({ "refusal": "document_not_granted", "document": request.document }),
            )
            .into();
        }
        // The message keeps the old wording so first-party wire bytes are
        // unchanged.
        if !self.grant.grants(cmd) {
            return Response::err(
                id,
                codes::NOT_PERMITTED,
                "method not in the granted profile",
            )
            .into();
        }
        let class = cmd.class;
        let name = cmd.name.as_str();
        if self.grant.is_restricted()
            && let Some(pointer) = crate::paths::path_param_set(
                name,
                request.params.as_ref().unwrap_or(&serde_json::Value::Null),
            )
        {
            return Response::err_with(
                id,
                codes::NOT_PERMITTED,
                "path parameters are not granted to this connection",
                serde_json::json!({ "refusal": "path_not_granted", "method": name, "pointer": pointer }),
            )
            .into();
        }
        // The `document` parameter of the window commands is the same
        // reach as the envelope member above.
        if self.grant.is_restricted()
            && !self.grant.may_address_other_documents()
            && let Some(document) = crate::paths::document_param_set(
                name,
                request.params.as_ref().unwrap_or(&serde_json::Value::Null),
            )
        {
            return Response::err_with(
                id,
                codes::NOT_PERMITTED,
                "document not granted to this connection",
                serde_json::json!({ "refusal": "document_not_granted", "document": document }),
            )
            .into();
        }
        // A request that names its document (§4.2's envelope `document`)
        // addresses it directly: it must resolve, and it needs no prior
        // attach. A host with several documents routes before the request
        // ever reaches this dispatcher (the desktop shell strips the
        // member), so what arrives here names the one document this
        // dispatcher has.
        let explicit = match &request.document {
            None => false,
            Some(query) => {
                let documents = host.documents(doc);
                if let Err(refusal) = crate::host::resolve_document(&documents, query) {
                    return Response::err_with(
                        id,
                        codes::REFUSED,
                        "refused",
                        refusal.into_data(0, name),
                    )
                    .into();
                }
                true
            }
        };
        // Inside a read scope nothing may change the document; a
        // transaction is judged by what it contains (below).
        if self.read_scope_depth > 0 && cmd.mutates_document && name != "hew.doc.transact" {
            return read_only_forward(id).into();
        }
        // One computation at a time per connection, refused before any
        // work; the guard releases the gate on every exit path.
        let _compute = match compute_in(name, request.params.as_ref()) {
            Some((index, method)) => match self.compute_gate.try_acquire() {
                Some(guard) => Some(guard),
                None => return compute_busy(id, index, &method).into(),
            },
            None => None,
        };
        // A plugin-provided command never executes here: it is validated
        // and handed back for its plugin (docs/agents/HEW_API.md §10.2).
        if cmd.is_plugin() {
            if !self.attached && !explicit {
                return Response::err(id, codes::NO_DOCUMENT, "no document attached").into();
            }
            let params = request.params.unwrap_or(serde_json::json!({}));
            return self.forward(cmd, id, params, None);
        }
        Replied::Response(match name {
            "hew.meta.hello" => self.hello(id, request.params, doc, host),
            "hew.meta.capabilities" => self.capabilities(registry, id),
            "hew.event.subscribe" | "hew.event.unsubscribe" => {
                let params = request.params.unwrap_or(serde_json::json!({}));
                match self.subscribe(name == "hew.event.subscribe", &params) {
                    Ok(result) => Response::ok(id, result),
                    Err(msg) => Response::err(id, codes::INVALID_PARAMS, &msg),
                }
            }
            "hew.doc.attach" => {
                let params = request.params.unwrap_or(serde_json::json!({}));
                match self.attach(doc, host, &params) {
                    Ok(result) => Response::ok(id, result),
                    Err(AttachError::Params(msg)) => Response::err(id, codes::INVALID_PARAMS, &msg),
                    Err(AttachError::Refused(refusal)) => Response::err_with(
                        id,
                        codes::REFUSED,
                        "refused",
                        refusal.into_data(0, "hew.doc.attach"),
                    ),
                }
            }
            "hew.doc.transact" => {
                let params = request.params.unwrap_or(serde_json::json!({}));
                // A dry run is answered whole here, success or failure, as
                // its own outcome, so the host skips everything it does
                // after a mutation (docs/agents/HEW_API.md §6.6).
                // The envelope's own checks come first, as for a commit
                // (below), and only then the attachment check.
                if params.get("dry_run") == Some(&serde_json::Value::Bool(true)) {
                    if let Ok(tx) = serde_json::from_value::<transact::TxParams>(params.clone())
                        && let Err(e) = transact::precheck(doc, &tx)
                    {
                        return Replied::DryRun(tx_error_response(id, e, &params));
                    }
                    let response = if !self.attached && !explicit {
                        Response::err(id, codes::NO_DOCUMENT, "no document attached")
                    } else {
                        match transact::run_dry(doc, host, registry, &self.grant, &params) {
                            Ok(result) => Response::ok(id, result),
                            Err(e) => tx_error_response(id, e, &params),
                        }
                    };
                    return Replied::DryRun(response);
                }
                // The envelope's own checks (`expect_generation`, the
                // `coalesce` bounds) come first, for every envelope shape:
                // a plugin command's, the inline attach, and the rest,
                // before the read-scope and attachment checks (HEW_API.md
                // §6.5). A malformed envelope is left to the paths below.
                if let Ok(tx) = serde_json::from_value::<transact::TxParams>(params.clone())
                    && let Err(e) = transact::precheck(doc, &tx)
                {
                    return Replied::Response(tx_error_response(id, e, &params));
                }
                // A one-command envelope holding a plugin command is that
                // command's MCP invocation: it unwraps to the same forward
                // a plain request makes, answered in the transaction shape.
                if let Some((decl, inner)) = single_plugin_method(registry, &params) {
                    return self.forward_sole_command(decl, inner, id, explicit);
                }
                if self.read_scope_depth > 0 && transaction_mutates(registry, &params) {
                    return read_only_forward(id).into();
                }
                // A one-command envelope holding a solitary command is that
                // command's canonical MCP invocation (§6.4, §13); the
                // attachment-establishing ones keep their exemption and
                // auto-attach on success, and `attach` itself is served
                // inline here (its state lives on the connection).
                let single_solitary = single_solitary_method(registry, &params);
                if single_solitary.as_deref() == Some("hew.doc.attach") {
                    // Served inline, so the grant check `transact::run`
                    // would make happens here.
                    let decl = registry.get("hew.doc.attach").expect("registered");
                    if !self.grant.grants(decl) {
                        return Response::err_with(
                            id,
                            codes::NOT_PERMITTED,
                            "method not in the granted profile",
                            serde_json::json!({ "index": 0 }),
                        )
                        .into();
                    }
                    let attach_params = params
                        .pointer("/commands/0/params")
                        .cloned()
                        .unwrap_or(serde_json::json!({}));
                    return Replied::Response(match self.attach(doc, host, &attach_params) {
                        Ok(result) => Response::ok(
                            id,
                            serde_json::json!({ "results": [result], "label": "hew.doc.attach" }),
                        ),
                        Err(AttachError::Params(msg)) => {
                            Response::err(id, codes::INVALID_PARAMS, &msg)
                        }
                        Err(AttachError::Refused(refusal)) => Response::err_with(
                            id,
                            codes::REFUSED,
                            "refused",
                            refusal.into_data(0, "hew.doc.attach"),
                        ),
                    });
                }
                let establishes = matches!(
                    single_solitary.as_deref(),
                    Some("hew.doc.new") | Some("hew.doc.open")
                );
                if !self.attached && !establishes && !explicit {
                    return Response::err(id, codes::NO_DOCUMENT, "no document attached").into();
                }
                let response = self.tx_response(registry, id, doc, host, &params);
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
                if !(self.attached || establishes || host_wide || explicit) {
                    return Response::err(id, codes::NO_DOCUMENT, "no document attached").into();
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
                        let response = self.tx_response(registry, id, doc, host, &tx);
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
        })
    }

    /// Validates a plugin command's params and issues its forward
    /// (`wrapped` names the label of the sole-command transaction it came
    /// in, if any).
    fn forward(
        &mut self,
        cmd: &CommandDecl,
        id: Option<RequestId>,
        params: serde_json::Value,
        wrapped: Option<String>,
    ) -> Replied {
        let Some(plugin) = cmd.plugin_id() else {
            return Response::err(id, codes::INTERNAL_FAULT, "not a plugin command").into();
        };
        if let Err(why) = crate::schema_check::check(&cmd.params_schema, &params) {
            return Response::err(
                id,
                codes::INVALID_PARAMS,
                &format!("params do not match {}'s declared schema: {why}", cmd.name),
            )
            .into();
        }
        let Some(request_id) = id else {
            return Response::err(None, codes::INTERNAL_FAULT, "a forward needs a request id")
                .into();
        };
        let token = NEXT_FORWARD_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.pending_forwards.insert(
            token,
            PendingForward {
                request_id: request_id.clone(),
                name: cmd.name.clone(),
                result_schema: cmd.result_schema.clone(),
                plugin: plugin.to_string(),
                wrapped,
            },
        );
        Replied::Forward(ForwardRequest {
            token,
            plugin: plugin.to_string(),
            local: cmd.name[plugin.len() + 1..].to_string(),
            method: cmd.name.clone(),
            request_id,
            params,
            mutating: cmd.mutates_document,
            label: cmd.label.clone(),
            timeout_ms: cmd.timeout_ms,
            concurrency: cmd.concurrency,
            caller: ForwardCaller {
                identity: self.identity.clone(),
                plugin: self.grant.plugin.clone(),
                app: self.grant.app.clone(),
                forward: self.active_forward,
            },
        })
    }

    /// A `hew.doc.transact` whose one command is a plugin command: the
    /// checks `transact::run` would make for it, then the same forward a
    /// plain request makes, answered later in the transaction shape.
    fn forward_sole_command(
        &mut self,
        decl: &CommandDecl,
        tx: transact::TxParams,
        id: Option<RequestId>,
        explicit: bool,
    ) -> Replied {
        // The envelope's own checks ran first (`reply`). A plugin command's
        // undo entry is its compound, so there is nothing for a key to
        // extend.
        if tx.coalesce.is_some() {
            return Replied::Response(tx_error_response(
                id,
                TxError::Invalid(
                    "coalesce does not apply when the sole command is a plugin command; its undo entry is that command's compound"
                        .into(),
                ),
                &serde_json::Value::Null,
            ));
        }
        let command = &tx.commands[0];
        if !self.grant.grants(decl) {
            return Response::err_with(
                id,
                codes::NOT_PERMITTED,
                "method not in the granted profile",
                serde_json::json!({ "index": 0 }),
            )
            .into();
        }
        if let Err(e) = transact::static_checks(&self.grant, &command.method, &command.params, 0) {
            return Replied::Response(tx_error_response(id, e, &serde_json::Value::Null));
        }
        if self.read_scope_depth > 0 && decl.mutates_document {
            return read_only_forward(id).into();
        }
        if !self.attached && !explicit {
            return Response::err(id, codes::NO_DOCUMENT, "no document attached").into();
        }
        let label = tx
            .label
            .clone()
            .or_else(|| decl.label.clone())
            .unwrap_or_else(|| decl.name.clone());
        let params = command.params.clone().unwrap_or(serde_json::json!({}));
        self.forward(decl, id, params, Some(label))
    }

    /// Shapes the final response for a forward this connection issued,
    /// from the plugin's reply (docs/agents/HEW_API.md §10.2). Each token
    /// completes once; an unknown or already-completed token is `-32003`.
    pub fn complete_forward(
        &mut self,
        registry: &Registry,
        token: u64,
        reply: ForwardReply,
    ) -> Response {
        // The registry is not consulted today: everything the shaping
        // needs was captured when the forward was issued, so a plugin
        // unregistered meanwhile still gets its answer delivered.
        let _ = registry;
        let Some(pending) = self.pending_forwards.remove(&token) else {
            return Response::err(None, codes::INTERNAL_FAULT, "unknown forward token");
        };
        let id = Some(pending.request_id);
        let method = pending.name.as_str();
        let plugin = pending.plugin.as_str();
        let refused = |refusal: Refusal| {
            Response::err_with(
                id.clone(),
                codes::REFUSED,
                "refused",
                refusal.into_data(0, method),
            )
        };
        match reply {
            ForwardReply::Ok(value) => {
                match crate::schema_check::check(&pending.result_schema, &value) {
                    Ok(()) => match pending.wrapped {
                        Some(label) => Response::ok(
                            id,
                            serde_json::json!({ "results": [value], "label": label }),
                        ),
                        None => Response::ok(id, value),
                    },
                    Err(why) => Response::err_with(
                        id,
                        codes::INTERNAL_FAULT,
                        "plugin returned a result that does not match its declared schema",
                        serde_json::json!({
                            "refusal": "plugin_bad_result",
                            "plugin": plugin,
                            "method": method,
                            "why": why,
                            "raw": value,
                        }),
                    ),
                }
            }
            ForwardReply::Refusal {
                name,
                explanation,
                detail,
            } => refused(Refusal {
                name: format!("{plugin}.{name}"),
                explanation,
                detail: if detail.is_null() {
                    serde_json::json!({})
                } else {
                    detail
                },
            }),
            ForwardReply::Failed(ForwardFailure::Timeout) => Response::err_with(
                id,
                codes::INTERNAL_FAULT,
                "the plugin did not answer in time",
                serde_json::json!({ "plugin": plugin, "method": method, "reason": "timeout" }),
            ),
            ForwardReply::Failed(ForwardFailure::Crashed) => Response::err_with(
                id,
                codes::INTERNAL_FAULT,
                "the plugin stopped while handling this command",
                serde_json::json!({ "plugin": plugin, "method": method, "reason": "crashed" }),
            ),
            ForwardReply::Failed(ForwardFailure::Cancelled) => refused(Refusal::api(
                "plugin_command_cancelled",
                "The user cancelled this command while it was running.",
            )),
            ForwardReply::Failed(ForwardFailure::Unavailable) => refused(
                Refusal::api(
                    "plugin_unavailable",
                    "This plugin is installed but not running. Enable it in Settings ▸ Plugins, or reload it.",
                )
                .with_detail(serde_json::json!({ "plugin": plugin })),
            ),
            ForwardReply::Failed(ForwardFailure::NoHandler) => Response::err_with(
                id,
                codes::METHOD_NOT_FOUND,
                "the plugin registers no handler for this command",
                serde_json::json!({ "plugin": plugin, "method": method }),
            ),
        }
    }

    /// Executes a transaction envelope and shapes its response.
    fn tx_response(
        &mut self,
        registry: &Registry,
        id: Option<RequestId>,
        doc: &mut kernel::Document,
        host: &mut dyn Host,
        params: &serde_json::Value,
    ) -> Response {
        match transact::run(doc, host, registry, &self.grant, &self.identity, params) {
            Ok(result) => Response::ok(id, result),
            Err(e) => tx_error_response(id, e, params),
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
            grant: &self.grant,
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

    /// `hew.event.subscribe` (`add`) and `hew.event.unsubscribe`: `events`
    /// is 1 to 32 names from [`crate::events::EVENT_NAMES`]. Subscribing
    /// adds each name the grant may receive and reports the rest in
    /// `denied`; unsubscribing removes them. Either way `subscribed` is the
    /// whole set afterwards, in name order. A malformed list or an unknown
    /// name changes nothing.
    fn subscribe(
        &mut self,
        add: bool,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let Some(object) = params.as_object() else {
            return Err("params must be an object".into());
        };
        if let Some(extra) = object.keys().find(|k| k.as_str() != "events") {
            return Err(format!("unknown field `{extra}`"));
        }
        let Some(list) = object.get("events").and_then(serde_json::Value::as_array) else {
            return Err("`events` must be an array of event names".into());
        };
        if list.is_empty() || list.len() > 32 {
            return Err("`events` must name 1 to 32 events".into());
        }
        let mut names = Vec::with_capacity(list.len());
        for v in list {
            let Some(name) = v.as_str() else {
                return Err("`events` must be an array of event names".into());
            };
            if !crate::events::EVENT_NAMES.contains(&name) {
                return Err(format!("unknown event name: {name}"));
            }
            names.push(name);
        }
        let mut denied: Vec<&str> = Vec::new();
        for name in names {
            if !add {
                self.subscriptions.remove(name);
            } else if self.grant.may_receive(name) {
                self.subscriptions.insert(name.to_string());
            } else if !denied.contains(&name) {
                denied.push(name);
            }
        }
        Ok(serde_json::json!({
            "subscribed": self.subscriptions.iter().collect::<Vec<_>>(),
            "denied": denied,
        }))
    }

    /// `hew.doc.attach`: bind this connection to the document named by
    /// `document` (resolved against the host's open documents), or — with
    /// no `document` — to the host's current one. Single-document hosts
    /// attach with no params; a named document that does not resolve is
    /// refused typed, never silently accepted, so a client can never go
    /// on to edit a window it did not mean to.
    fn attach(
        &mut self,
        doc: &kernel::Document,
        host: &mut dyn Host,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, AttachError> {
        let Some(object) = params.as_object() else {
            return Err(AttachError::Params("params must be an object".into()));
        };
        if let Some(extra) = object.keys().find(|k| k.as_str() != "document") {
            return Err(AttachError::Params(format!("unknown field `{extra}`")));
        }
        let mut result = serde_json::json!({});
        match object.get("document") {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(_))
                if self.grant.is_restricted() && !self.grant.may_address_other_documents() =>
            {
                return Err(AttachError::Refused(crate::refusal::Refusal::api(
                    "document_not_granted",
                    "This connection is bound to one document and may not attach to another. Ask the user for documents.other.",
                )));
            }
            Some(serde_json::Value::String(query)) => {
                let documents = host.documents(doc);
                let found = crate::host::resolve_document(&documents, query)
                    .map_err(AttachError::Refused)?;
                result = serde_json::json!({ "document": found });
            }
            Some(_) => {
                return Err(AttachError::Params("`document` must be a string".into()));
            }
        }
        self.attached = true;
        Ok(result)
    }

    fn hello(
        &mut self,
        id: Option<RequestId>,
        params: Option<serde_json::Value>,
        doc: &kernel::Document,
        host: &dyn Host,
    ) -> Response {
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
        let restricted = self.grant.is_restricted();
        let mut result = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "app": { "name": "hew", "version": env!("CARGO_PKG_VERSION") },
            "profile": match (restricted, self.grant.ceiling) {
                (true, _) => "plugin",
                (false, crate::grant::Ceiling::Core) => "core",
                (false, crate::grant::Ceiling::App) => "app",
            },
            "encoding": "json",
            "documents": self.grant.filter_documents(host.documents(doc)),
        });
        if restricted {
            result["grant"] = serde_json::to_value(&self.grant).expect("a grant serializes");
        }
        Response::ok(id, result)
    }

    /// A command's refusal names as this connection's capabilities show
    /// them: a first-party connection does not see the names only a
    /// restricted grant can be refused with, so its capabilities stay
    /// exactly what they were before grants existed.
    fn visible_refusals<'a>(&self, refusals: &'a [String]) -> Vec<&'a str> {
        refusals
            .iter()
            .map(String::as_str)
            .filter(|r| self.grant.is_restricted() || !RESTRICTED_ONLY_REFUSALS.contains(r))
            .collect()
    }

    fn capabilities(&self, registry: &Registry, id: Option<RequestId>) -> Response {
        let commands: Vec<serde_json::Value> = registry
            .commands()
            .filter(|c| self.grant.grants(c))
            .map(|c| {
                let mut entry = serde_json::json!({
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
                    "refusals": self.visible_refusals(&c.refusals),
                    "implemented": c.implemented,
                });
                // A plugin entry always says who provides it and how it is
                // called; a restricted grant sees provider and permission
                // on every entry. First-party capabilities of first-party
                // commands are exactly what they were before plugins.
                if let Provider::Plugin(plugin) = &c.provider {
                    entry["provider"] = serde_json::json!({ "plugin": plugin });
                    entry["permission"] = serde_json::Value::String(c.permission.to_wire());
                    entry["transactable"] = serde_json::Value::Bool(false);
                    entry["label"] = serde_json::json!(c.label);
                    entry["timeout_ms"] = serde_json::json!(c.timeout_ms);
                    entry["concurrency"] = serde_json::json!(c.concurrency);
                } else if self.grant.is_restricted() {
                    entry["provider"] = serde_json::Value::String(
                        match c.provider {
                            Provider::Host => "host",
                            _ => "kernel",
                        }
                        .into(),
                    );
                    entry["permission"] = serde_json::Value::String(c.permission.to_wire());
                }
                entry
            })
            .collect();
        let mut result = serde_json::json!({ "commands": commands });
        if self.grant.is_restricted() {
            result["grant"] = serde_json::to_value(&self.grant).expect("a grant serializes");
        }
        Response::ok(id, result)
    }
}

/// Refusal names only a restricted grant (a plugin or a connected app) can
/// be answered with. Registry declarations list them; first-party
/// capabilities leave them out.
pub const RESTRICTED_ONLY_REFUSALS: &[&str] =
    &["document_not_granted", "attr_namespace_not_granted"];

/// `-32001` `read_only_forward`: a document change asked for inside a read
/// scope.
fn read_only_forward(id: Option<RequestId>) -> Response {
    Response::err_with(
        id,
        codes::NOT_PERMITTED,
        "this command is not available while serving a read-only command",
        serde_json::json!({ "refusal": "read_only_forward" }),
    )
}

/// The response for a failed transaction. `params` is the envelope, read
/// back for the method a refusal names.
fn tx_error_response(id: Option<RequestId>, e: TxError, params: &serde_json::Value) -> Response {
    match e {
        TxError::Invalid(msg) => Response::err(id, codes::INVALID_PARAMS, &msg),
        TxError::NotPermitted(index) => Response::err_with(
            id,
            codes::NOT_PERMITTED,
            "method not in the granted profile",
            serde_json::json!({ "index": index }),
        ),
        TxError::PathNotGranted { index, pointer } => Response::err_with(
            id,
            codes::NOT_PERMITTED,
            "path parameters are not granted to this connection",
            serde_json::json!({ "refusal": "path_not_granted", "index": index, "pointer": pointer }),
        ),
        TxError::DocumentNotGranted { index, document } => Response::err_with(
            id,
            codes::NOT_PERMITTED,
            "document not granted to this connection",
            serde_json::json!({ "refusal": "document_not_granted", "index": index, "document": document }),
        ),
        TxError::PluginCommandInTransaction(index) => {
            let method = params
                .pointer(&format!("/commands/{index}/method"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            Response::err_with(
                id,
                codes::REFUSED,
                "refused",
                Refusal::api(
                    "plugin_command_in_transaction",
                    "A plugin command runs as its own envelope; it cannot share a transaction with other commands or with another plugin command.",
                )
                .into_data(index, method),
            )
        }
        TxError::Refused {
            index,
            method,
            refusal,
        } => Response::err_with(
            id,
            codes::REFUSED,
            "refused",
            refusal.into_data(index, &method),
        ),
        TxError::Internal(msg) => Response::err(id, codes::INTERNAL_FAULT, &msg),
    }
}

/// The plugin command of a one-command transact envelope, with the parsed
/// envelope — `None` for every other envelope shape.
fn single_plugin_method<'r>(
    registry: &'r Registry,
    params: &serde_json::Value,
) -> Option<(&'r CommandDecl, transact::TxParams)> {
    let tx: transact::TxParams = serde_json::from_value(params.clone()).ok()?;
    if tx.commands.len() != 1 {
        return None;
    }
    let decl = registry.get(&tx.commands[0].method)?;
    decl.is_plugin().then_some((decl, tx))
}

/// Whether any command of a transact envelope can change the document.
/// Unknown methods count as not, and are left to the static pass.
fn transaction_mutates(registry: &Registry, params: &serde_json::Value) -> bool {
    params
        .get("commands")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|commands| {
            commands.iter().any(|c| {
                c.get("method")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|m| registry.get(m))
                    .is_some_and(|d| d.mutates_document)
            })
        })
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
            document: None,
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(id)),
            method: method.to_string(),
            params: Some(params),
        }
    }

    fn hello(conn: &mut Connection, doc: &mut kernel::Document) {
        let out = conn.dispatch(
            &Registry::protocol_1(),
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
        let mut conn = Connection::new(Grant::core(), "test");
        let mut doc = kernel::Document::new();
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &Registry::protocol_1(),
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
        let mut conn = Connection::new(Grant::core(), "test");
        let mut doc = kernel::Document::new();
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &Registry::protocol_1(),
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
    /// (`grant.rs`'s `core_and_app_match_the_old_profile_rule`).
    #[test]
    fn core_now_reaches_the_host_for_snapshot_instead_of_not_permitted() {
        let mut conn = Connection::new(Grant::core(), "test");
        let mut doc = kernel::Document::new();
        hello(&mut conn, &mut doc);
        conn.dispatch(
            &Registry::protocol_1(),
            &mut doc,
            &mut crate::host::NoHost,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &Registry::protocol_1(),
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
        let mut conn = Connection::new(Grant::core(), "test");
        let mut doc = kernel::Document::new();
        hello(&mut conn, &mut doc);
        conn.dispatch(
            &Registry::protocol_1(),
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
        let mut conn = Connection::new(Grant::core(), "test");
        let mut doc = kernel::Document::new();
        hello(&mut conn, &mut doc);
        let out = conn.dispatch(
            &Registry::protocol_1(),
            &mut doc,
            &mut crate::host::NoHost,
            Request {
                document: None,
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
        let mut conn = Connection::new(Grant::core(), "test");
        let mut doc = kernel::Document::new();
        // Wrong protocol refused with the versions we speak.
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &Registry::protocol_1(),
            &mut doc,
            &mut crate::host::NoHost,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 99})),
        ) else {
            panic!()
        };
        assert_eq!(r.error.as_ref().unwrap().code, codes::INVALID_PARAMS);

        hello(&mut conn, &mut doc);
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &Registry::protocol_1(),
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

    // ------------------------------------------------ plugin forwarding

    /// `com.example.lib`: read-only `sum` (closed params and result,
    /// refusal `too_big`) and mutating `mut` (label "Add box").
    fn plugin_registry() -> Registry {
        let decls: Vec<plugin_manifest::CommandContribution> = serde_json::from_value(serde_json::json!([
            {
                "id": "sum", "class": "read_only", "summary": "Adds.",
                "params": { "type": "object", "properties": { "a": { "type": "number" }, "b": { "type": "number" } }, "required": ["a", "b"], "additionalProperties": false },
                "result": { "type": "object", "properties": { "sum": { "type": "number" } }, "required": ["sum"], "additionalProperties": false },
                "refusals": ["too_big"]
            },
            { "id": "mut", "class": "mutating", "summary": "Mutates.", "label": "Add box" }
        ]))
        .unwrap();
        let mut registry = Registry::protocol_1();
        registry.register_plugin("com.example.lib", &decls).unwrap();
        registry
    }

    /// A hello'd, attached first-party connection.
    fn ready(registry: &Registry, doc: &mut kernel::Document) -> Connection {
        let mut conn = Connection::new(Grant::core(), "test");
        for (i, method) in [(0, "hew.meta.hello"), (1, "hew.doc.attach")] {
            let params = if i == 0 {
                serde_json::json!({"protocol": 1})
            } else {
                serde_json::json!({})
            };
            let out = conn.dispatch(
                registry,
                doc,
                &mut crate::host::NoHost,
                req(i, method, params),
            );
            assert!(matches!(out, DispatchOutcome::Reply(ref r) if r.error.is_none()));
        }
        conn
    }

    fn forwarded(out: DispatchOutcome) -> ForwardRequest {
        match out {
            DispatchOutcome::Forward(f) => f,
            other => panic!("expected a forward, got {other:?}"),
        }
    }

    fn replied(out: DispatchOutcome) -> Response {
        match out {
            DispatchOutcome::Reply(r) => r,
            other => panic!("expected a reply, got {other:?}"),
        }
    }

    #[test]
    fn plugin_command_forwards_with_validated_params() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        // Unattached: -32002 before anything is forwarded.
        let mut fresh = Connection::new(Grant::core(), "test");
        fresh.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        let r = replied(fresh.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                1,
                "com.example.lib.sum",
                serde_json::json!({"a": 1, "b": 2}),
            ),
        ));
        assert_eq!(r.error.unwrap().code, codes::NO_DOCUMENT);
        assert_eq!(fresh.pending_forward_count(), 0);

        let mut conn = ready(&registry, &mut doc);
        let f = forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                7,
                "com.example.lib.sum",
                serde_json::json!({"a": 1, "b": 2}),
            ),
        ));
        assert_eq!(f.plugin, "com.example.lib");
        assert_eq!(f.local, "sum");
        assert_eq!(f.method, "com.example.lib.sum");
        assert_eq!(f.request_id, RequestId::Number(7));
        assert_eq!(f.params, serde_json::json!({"a": 1, "b": 2}));
        assert!(!f.mutating);
        assert_eq!(f.label, None);
        assert_eq!(f.timeout_ms, 50_000);
        assert_eq!(f.caller.identity, "test");
        assert_eq!(f.caller.forward, None);
        assert_eq!(conn.pending_forward_count(), 1);

        let m = forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(8, "com.example.lib.mut", serde_json::json!({})),
        ));
        assert!(m.mutating);
        assert_eq!(m.label.as_deref(), Some("Add box"));
        assert_ne!(m.token, f.token, "tokens are never reused");

        // Absent params are the empty object, which `sum` refuses.
        let mut absent = req(9, "com.example.lib.sum", serde_json::json!({}));
        absent.params = None;
        let r = replied(conn.dispatch(&registry, &mut doc, &mut crate::host::NoHost, absent));
        let e = r.error.unwrap();
        assert_eq!(e.code, codes::INVALID_PARAMS);
        assert_eq!(
            e.message,
            "params do not match com.example.lib.sum's declared schema: /: missing required property \"a\""
        );
        assert_eq!(
            conn.pending_forward_count(),
            2,
            "a refused call issues no forward"
        );
    }

    #[test]
    fn sole_plugin_transact_unwraps_and_rewraps() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        let tx = |label: Option<&str>, params: serde_json::Value| {
            let mut t = serde_json::json!({ "commands": [{ "method": "com.example.lib.sum", "params": params }] });
            if let Some(l) = label {
                t["label"] = l.into();
            }
            t
        };
        let f = forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                3,
                "hew.doc.transact",
                tx(None, serde_json::json!({"a": 2, "b": 2})),
            ),
        ));
        assert_eq!(f.params, serde_json::json!({"a": 2, "b": 2}));
        let r = conn.complete_forward(
            &registry,
            f.token,
            ForwardReply::Ok(serde_json::json!({"sum": 4})),
        );
        assert_eq!(r.id, Some(RequestId::Number(3)));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"results": [{"sum": 4}], "label": "com.example.lib.sum"})
        );

        // The envelope's own label wins; the declared label is next.
        let f = forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                4,
                "hew.doc.transact",
                tx(Some("mine"), serde_json::json!({"a": 1, "b": 1})),
            ),
        ));
        let r = conn.complete_forward(
            &registry,
            f.token,
            ForwardReply::Ok(serde_json::json!({"sum": 2})),
        );
        assert_eq!(r.result.unwrap()["label"], "mine");
        let f = forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                5,
                "hew.doc.transact",
                serde_json::json!({"commands": [{"method": "com.example.lib.mut"}]}),
            ),
        ));
        let r = conn.complete_forward(&registry, f.token, ForwardReply::Ok(serde_json::json!({})));
        assert_eq!(r.result.unwrap()["label"], "Add box");

        // Inner params are schema-checked too, and a `$ref` in a sole
        // command can name nothing.
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                6,
                "hew.doc.transact",
                tx(None, serde_json::json!({"a": "x", "b": 1})),
            ),
        ));
        assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                7,
                "hew.doc.transact",
                tx(None, serde_json::json!({"a": {"$ref": "x#/y"}, "b": 1})),
            ),
        ));
        assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);

        // A grant without model.write may not reach `mut` this way either.
        let restricted = Grant::plugin(
            "com.example.caller",
            None,
            [crate::grant::Permission::ModelRead].into(),
        );
        let mut plugin_conn = Connection::new(restricted, "plugin:com.example.caller");
        plugin_conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        plugin_conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );
        let r = replied(plugin_conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                2,
                "hew.doc.transact",
                serde_json::json!({"commands": [{"method": "com.example.lib.mut"}]}),
            ),
        ));
        let e = r.error.unwrap();
        assert_eq!(e.code, codes::NOT_PERMITTED);
        assert_eq!(e.data, Some(serde_json::json!({"index": 0})));
    }

    #[test]
    fn plugin_command_in_mixed_transaction_is_refused_statically() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        let before = doc.history_generation();
        for (commands, index, method) in [
            (
                serde_json::json!([{"method": "com.example.lib.sum", "params": {"a": 1, "b": 1}}, {"method": "hew.attr.get", "params": {"target": "document"}}]),
                0,
                "com.example.lib.sum",
            ),
            (
                serde_json::json!([{"method": "hew.attr.get", "params": {"target": "document"}}, {"method": "com.example.lib.mut"}]),
                1,
                "com.example.lib.mut",
            ),
            (
                serde_json::json!([{"method": "com.example.lib.sum", "params": {"a": 1, "b": 1}}, {"method": "com.example.lib.sum", "params": {"a": 1, "b": 1}}]),
                0,
                "com.example.lib.sum",
            ),
        ] {
            let r = replied(conn.dispatch(
                &registry,
                &mut doc,
                &mut crate::host::NoHost,
                req(
                    2,
                    "hew.doc.transact",
                    serde_json::json!({"commands": commands}),
                ),
            ));
            let e = r.error.unwrap();
            assert_eq!(e.code, codes::REFUSED);
            let data = e.data.unwrap();
            assert_eq!(data["refusal"], "plugin_command_in_transaction");
            assert_eq!(data["failed_index"], index);
            assert_eq!(data["failed_method"], method);
        }
        assert_eq!(conn.pending_forward_count(), 0);
        assert_eq!(doc.history_generation(), before);
    }

    #[test]
    fn complete_forward_shapes_every_reply_kind() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        let mut run = |conn: &mut Connection, reply: serde_json::Value| -> serde_json::Value {
            let f = forwarded(conn.dispatch(
                &registry,
                &mut doc,
                &mut crate::host::NoHost,
                req(
                    5,
                    "com.example.lib.sum",
                    serde_json::json!({"a": 1, "b": 2}),
                ),
            ));
            let reply: ForwardReply = serde_json::from_value(reply).expect("wire form parses");
            serde_json::to_value(conn.complete_forward(&registry, f.token, reply)).unwrap()
        };
        let base = "com.example.lib";
        let method = "com.example.lib.sum";
        assert_eq!(
            run(&mut conn, serde_json::json!({"ok": {"sum": 3}})),
            serde_json::json!({"jsonrpc": "2.0", "id": 5, "result": {"sum": 3}})
        );
        assert_eq!(
            run(&mut conn, serde_json::json!({"ok": {"sum": "three"}}))["error"],
            serde_json::json!({"code": -32003, "message": "plugin returned a result that does not match its declared schema", "data": {"refusal": "plugin_bad_result", "plugin": base, "method": method, "why": "/sum: expected number, found string", "raw": {"sum": "three"}}})
        );
        assert_eq!(
            run(
                &mut conn,
                serde_json::json!({"refusal": {"name": "too_big", "explanation": "Too big.", "detail": {"limit": 10}}})
            )["error"],
            serde_json::json!({"code": -32000, "message": "refused", "data": {"refusal": "com.example.lib.too_big", "failed_index": 0, "failed_method": method, "detail": {"limit": 10}, "explanation": "Too big."}})
        );
        assert_eq!(
            run(
                &mut conn,
                serde_json::json!({"refusal": {"name": "too_big", "explanation": "Too big."}})
            )["error"]["data"]["detail"],
            serde_json::json!({}),
            "an absent detail is the empty object, as every refusal's is"
        );
        assert_eq!(
            run(&mut conn, serde_json::json!({"failed": "timeout"}))["error"],
            serde_json::json!({"code": -32003, "message": "the plugin did not answer in time", "data": {"plugin": base, "method": method, "reason": "timeout"}})
        );
        assert_eq!(
            run(&mut conn, serde_json::json!({"failed": "crashed"}))["error"],
            serde_json::json!({"code": -32003, "message": "the plugin stopped while handling this command", "data": {"plugin": base, "method": method, "reason": "crashed"}})
        );
        assert_eq!(
            run(&mut conn, serde_json::json!({"failed": "cancelled"}))["error"],
            serde_json::json!({"code": -32000, "message": "refused", "data": {"refusal": "plugin_command_cancelled", "failed_index": 0, "failed_method": method, "detail": {}, "explanation": "The user cancelled this command while it was running."}})
        );
        assert_eq!(
            run(&mut conn, serde_json::json!({"failed": "unavailable"}))["error"],
            serde_json::json!({"code": -32000, "message": "refused", "data": {"refusal": "plugin_unavailable", "failed_index": 0, "failed_method": method, "detail": {"plugin": base}, "explanation": "This plugin is installed but not running. Enable it in Settings ▸ Plugins, or reload it."}})
        );
        assert_eq!(
            run(&mut conn, serde_json::json!({"failed": "no_handler"}))["error"],
            serde_json::json!({"code": -32601, "message": "the plugin registers no handler for this command", "data": {"plugin": base, "method": method}})
        );
        assert_eq!(conn.pending_forward_count(), 0);
    }

    #[test]
    fn read_scope_blocks_mutations() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        conn.begin_read_scope();
        conn.begin_read_scope();
        conn.end_read_scope();
        let blocked = |r: Response| {
            let e = r.error.expect("refused");
            assert_eq!(e.code, codes::NOT_PERMITTED);
            assert_eq!(
                e.message,
                "this command is not available while serving a read-only command"
            );
            assert_eq!(
                e.data,
                Some(serde_json::json!({"refusal": "read_only_forward"}))
            );
        };
        blocked(replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(2, "com.example.lib.mut", serde_json::json!({})),
        )));
        blocked(replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                3,
                "hew.doc.transact",
                serde_json::json!({"commands": [{"method": "com.example.lib.mut"}]}),
            ),
        )));
        blocked(replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(4, "hew.history.undo", serde_json::json!({})),
        )));
        blocked(replied(conn.dispatch(&registry, &mut doc, &mut crate::host::NoHost, req(5, "hew.doc.transact", serde_json::json!({"commands": [{"method": "hew.attr.get", "params": {"target": "document"}}, {"method": "hew.guide.point", "params": {"point": [0, 0, 0]}}]})))));
        // Reads still pass, plugin and first-party alike.
        forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                6,
                "com.example.lib.sum",
                serde_json::json!({"a": 1, "b": 1}),
            ),
        ));
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(7, "hew.query.scene", serde_json::json!({})),
        ));
        assert!(r.error.is_none());
        conn.end_read_scope();
        conn.end_read_scope(); // extra ends are harmless
        forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(8, "com.example.lib.mut", serde_json::json!({})),
        ));
    }

    #[test]
    fn document_replacing_commands_are_busy_while_a_compound_is_open() {
        let registry = Registry::protocol_1();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        let token = doc.begin_compound(kernel::CompoundMeta {
            label: "Parts".into(),
            origin: kernel::HistoryOrigin::Connection("plugin:com.example.lib".into()),
            coalesce: None,
        });
        let busy = |r: Response, method: &str| {
            let e = r.error.expect("refused");
            assert_eq!(e.code, codes::REFUSED);
            let data = e.data.expect("data");
            assert_eq!(data["refusal"], "document_busy");
            assert_eq!(data["failed_method"], method);
            assert_eq!(data["failed_index"], 0);
        };
        for (i, method) in [
            "hew.doc.open",
            "hew.doc.new",
            "hew.doc.revert",
            "hew.doc.close",
        ]
        .into_iter()
        .enumerate()
        {
            let params = if method == "hew.doc.open" {
                serde_json::json!({"path": "/nowhere.hew"})
            } else {
                serde_json::json!({})
            };
            busy(
                replied(conn.dispatch(
                    &registry,
                    &mut doc,
                    &mut crate::host::NoHost,
                    req(10 + i as i64, method, params.clone()),
                )),
                method,
            );
            // The same command as the sole command of a transaction.
            busy(
                replied(conn.dispatch(
                    &registry,
                    &mut doc,
                    &mut crate::host::NoHost,
                    req(
                        20 + i as i64,
                        "hew.doc.transact",
                        serde_json::json!({"commands": [{"method": method, "params": params}]}),
                    ),
                )),
                method,
            );
        }
        // Everything else still dispatches (the host queues; this is the
        // dispatcher's own rule only).
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(30, "hew.query.scene", serde_json::json!({})),
        ));
        assert!(r.error.is_none());
        doc.abort_compound(token).expect("abort");
        // With the compound gone, open reaches the host again.
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(31, "hew.doc.new", serde_json::json!({})),
        ));
        assert_eq!(
            r.error.unwrap().data.unwrap()["refusal"],
            "host_capability_missing"
        );
    }

    #[test]
    fn sole_plugin_transact_honors_expect_generation() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        let tx = |g: u64| serde_json::json!({"expect_generation": g, "commands": [{"method": "com.example.lib.mut"}]});
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(2, "hew.doc.transact", tx(5)),
        ));
        let data = r.error.unwrap().data.unwrap();
        assert_eq!(data["refusal"], "generation_moved");
        assert_eq!(data["failed_method"], "hew.doc.transact");
        assert_eq!(
            data["detail"],
            serde_json::json!({"expected": 5, "actual": 0})
        );
        assert_eq!(conn.pending_forward_count(), 0, "nothing forwarded");
        forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(3, "hew.doc.transact", tx(0)),
        ));
    }

    /// Every envelope shape gets the envelope checks first: the inline
    /// attach, and an unattached multi-command envelope, as a plugin
    /// command's does.
    #[test]
    fn every_envelope_shape_checks_generation_and_coalesce_first() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        let attach = |extra: serde_json::Value| {
            let mut tx =
                serde_json::json!({"commands": [{"method": "hew.doc.attach", "params": {}}]});
            tx.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            tx
        };
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                2,
                "hew.doc.transact",
                attach(serde_json::json!({"expect_generation": 3})),
            ),
        ));
        assert_eq!(
            r.error.unwrap().data.unwrap()["refusal"],
            "generation_moved"
        );
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                3,
                "hew.doc.transact",
                attach(serde_json::json!({"coalesce": ""})),
            ),
        ));
        assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);

        // Unattached: generation_moved, not no_document, as for a plugin
        // command.
        let mut fresh = Connection::new(Grant::core(), "test");
        fresh.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        for commands in [
            serde_json::json!([{"method": "hew.attr.get", "params": {"target": "document"}}, {"method": "hew.attr.get", "params": {"target": "document"}}]),
            serde_json::json!([{"method": "com.example.lib.mut"}]),
        ] {
            let r = replied(fresh.dispatch(
                &registry,
                &mut doc,
                &mut crate::host::NoHost,
                req(
                    4,
                    "hew.doc.transact",
                    serde_json::json!({"expect_generation": 3, "commands": commands}),
                ),
            ));
            assert_eq!(
                r.error.unwrap().data.unwrap()["refusal"],
                "generation_moved"
            );
        }
    }

    #[test]
    fn sole_plugin_transact_refuses_coalesce() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                2,
                "hew.doc.transact",
                serde_json::json!({"coalesce": "k", "commands": [{"method": "com.example.lib.mut"}]}),
            ),
        ));
        let e = r.error.unwrap();
        assert_eq!(e.code, codes::INVALID_PARAMS);
        assert_eq!(
            e.message,
            "coalesce does not apply when the sole command is a plugin command; its undo entry is that command's compound"
        );
        assert_eq!(conn.pending_forward_count(), 0);
    }

    #[test]
    fn dispatch_in_forward_stamps_the_forwards_it_issues() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        assert_eq!(conn.active_forward(), None);
        let f = forwarded(conn.dispatch_in_forward(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(2, "com.example.lib.mut", serde_json::json!({})),
            41,
        ));
        assert_eq!(f.caller.forward, Some(41));
        assert_eq!(conn.active_forward(), None, "the stamp lasts one dispatch");
        let f = forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(3, "com.example.lib.mut", serde_json::json!({})),
        ));
        assert_eq!(f.caller.forward, None);
    }

    #[test]
    fn transact_checks_generation_first_and_bounds_the_coalesce_key() {
        let registry = Registry::protocol_1();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        let attr = serde_json::json!({"method": "hew.attr.set", "params": {"target": "document", "ns": "com.example.t", "key": "k", "value": 1}});
        // A stale generation is refused even when the envelope is
        // otherwise invalid: nothing else is looked at first.
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                2,
                "hew.doc.transact",
                serde_json::json!({"expect_generation": 9, "commands": []}),
            ),
        ));
        let data = r.error.unwrap().data.unwrap();
        assert_eq!(data["refusal"], "generation_moved");
        assert_eq!(
            data["detail"],
            serde_json::json!({"expected": 9, "actual": 0})
        );
        for key in [String::new(), "x".repeat(65)] {
            let r = replied(conn.dispatch(
                &registry,
                &mut doc,
                &mut crate::host::NoHost,
                req(
                    3,
                    "hew.doc.transact",
                    serde_json::json!({"coalesce": key, "commands": [attr.clone()]}),
                ),
            ));
            let e = r.error.unwrap();
            assert_eq!(e.code, codes::INVALID_PARAMS);
            assert_eq!(e.message, "coalesce must be 1..=64 bytes");
        }
        assert_eq!(doc.undo_depth(), 0, "nothing ran");
        let r = replied(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                4,
                "hew.doc.transact",
                serde_json::json!({"coalesce": "x".repeat(64), "expect_generation": 0, "commands": [attr]}),
            ),
        ));
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            doc.peek_undo_meta().unwrap().coalesce.as_deref(),
            Some("x".repeat(64).as_str())
        );
    }

    /// Document-scoped events reach a restricted connection only for the
    /// document it is bound to; an unbound one (a connected app without
    /// `documents.other`) hears about none; `documents.other` and first
    /// party hear about every one.
    #[test]
    fn events_are_scoped_to_the_bound_document() {
        let registry = Registry::protocol_1();
        let read: std::collections::BTreeSet<crate::Permission> =
            [crate::Permission::ModelRead].into_iter().collect();
        let mut other = read.clone();
        other.insert(crate::Permission::DocumentsOther);
        let subscribed = |grant: Grant| {
            let mut doc = kernel::Document::new();
            let mut conn = Connection::new(grant, "t");
            hello(&mut conn, &mut doc);
            let out = conn.dispatch(
                &registry,
                &mut doc,
                &mut crate::NoHost,
                req(
                    5,
                    "hew.event.subscribe",
                    serde_json::json!({ "events": ["document_changed"] }),
                ),
            );
            assert!(matches!(out, DispatchOutcome::Reply(ref r) if r.error.is_none()));
            conn
        };
        let bound = subscribed(Grant::plugin("com.example.p", Some("doc_1"), read.clone()));
        assert!(bound.wants_for("document_changed", "doc_1"));
        assert!(!bound.wants_for("document_changed", "doc_2"));
        let unbound = subscribed(Grant::connected_app("pair-1", read));
        assert!(unbound.wants("document_changed"));
        assert!(!unbound.wants_for("document_changed", "doc_1"));
        let roaming = subscribed(Grant::plugin("com.example.p", Some("doc_1"), other));
        assert!(roaming.wants_for("document_changed", "doc_2"));
        assert!(subscribed(Grant::app()).wants_for("document_changed", "doc_9"));
    }

    #[test]
    fn unknown_token_is_internal_fault() {
        let registry = plugin_registry();
        let mut doc = kernel::Document::new();
        let mut conn = ready(&registry, &mut doc);
        let r = conn.complete_forward(&registry, 42, ForwardReply::Ok(serde_json::json!({})));
        assert_eq!(r.id, None);
        let e = r.error.unwrap();
        assert_eq!(e.code, codes::INTERNAL_FAULT);
        assert_eq!(e.message, "unknown forward token");
        // A completed token is spent.
        let f = forwarded(conn.dispatch(
            &registry,
            &mut doc,
            &mut crate::host::NoHost,
            req(
                2,
                "com.example.lib.sum",
                serde_json::json!({"a": 1, "b": 1}),
            ),
        ));
        assert!(
            conn.complete_forward(
                &registry,
                f.token,
                ForwardReply::Ok(serde_json::json!({"sum": 2}))
            )
            .error
            .is_none()
        );
        let again = conn.complete_forward(
            &registry,
            f.token,
            ForwardReply::Ok(serde_json::json!({"sum": 2})),
        );
        assert_eq!(again.error.unwrap().message, "unknown forward token");
    }
}
