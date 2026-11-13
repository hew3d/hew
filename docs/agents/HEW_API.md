# Hew — API Specification

> **Status: implemented (headless, protocol 1; live transport both sides
> on all three desktops).** The command surface, transactions, addressing,
> profiles, and conformance suite of this document are implemented in
> `crates/api`, with `hew-cli` as the first host (script runner, one-shot
> dispatch, MCP on stdio). `hew-cli`'s client side of `--live` (§11.2,
> §12) — discovery, the token-bearing handshake, and envelope forwarding
> over a Unix socket on macOS/Linux and a named pipe on Windows — is
> implemented and verified end to end against a running app on macOS and
> Windows 11. The desktop app's server side is implemented to match:
> `shells/tauri` binds the per-pid socket or pipe, publishes and sweeps
> the discovery file, and enforces the first-frame token gate shell-side; `crates/wasm-api` dispatches every
> forwarded frame against the SAME live `Document` the viewport renders,
> granted `Profile::App`, with the webview refreshing exactly as it does
> after any other kernel mutation. `hew.doc.save` and `hew.doc.export`
> both work live by handing bytes back for the caller to write: neither
> serializing the document nor writing STL/3MF/GLB (`crates/mesh-export`,
> shared by `crates/hew-cli`'s headless host and `crates/wasm-api`'s
> `LiveHost`) needs a disk, only the final write does — `hew-cli --live`
> does that write for you, so the same command produces the same file
> headless or live. The rest of the host-implemented commands
> (`hew.doc.new/open/import`) still refuse typed: document lifecycle stays
> user-driven, so a remote connection cannot silently replace or overwrite
> what the user has open. `hew.view.snapshot` and `hew.view.line_drawing` are
> rendered by the app itself, through the same `crates/headless-render`
> code `hew-cli` runs (§7.2), so both hosts return the same bytes for the
> same inputs; only a `path` write still needs a filesystem, which `hew-cli --live` does client-side: it asks the app for the bytes once, without `path`, writes the file and the companion pass files itself, and prints the headless result shape. And
> `hew.view.camera`, `hew.view.zoom_extents`, and `hew.view.units` — which
> need no bytes back, only an effect — ARE wired end to end: `LiveHost`
> records the requested effect and answers success, `crates/wasm-api`
> hands it to the webview's live bridge (`app/src/api/liveBridge.ts`) as a
> directive read out right after dispatch, and the bridge applies it
> through the Viewport's existing camera calls or
> `app/src/settings/units.ts`'s own setter — the same code paths the
> Camera menu and Settings window already use. So `--live` today is real
> for the kernel-served command surface (sketch/solid/structure/entity/
> style/attrs/history/scenes — the bulk of the protocol), for
> `save`/`export`, for these three view/display effects, and honestly
> refuses the remaining host-effect commands that need filesystem
> rendering access this sandbox does not have. `hew.scenes.apply`
> additionally leaves a `ViewDirective::ActivateScene` for the live
> bridge (§7.1) — real on the Rust/WASM side, though the app-side
> `LiveBridgeDeps.activateScene` hookup that would actually drive the
> UI from it is not yet wired. Sections marked *Reserved* remain
> deliberately unbuilt; per-command gaps are declared in the registry and
> answer typed refusals.

The Hew API is the single public, versioned command surface through which
any program other than Hew itself reads and edits a Hew document. Its first
clients are AI agents (through an MCP adapter) and shell scripts (through
`hew-cli`); its later clients are sandboxed plugins and, eventually, the
domain overlays described in the roadmap. All of them speak the same
protocol and differ only in which subset of it — which *profile* — a host
grants them.

This document is written for implementers of the protocol and of clients
that must match it exactly. If you are writing a client, start with
API_GUIDE.md, which covers connecting, the shape of a session, and worked
examples of every idiom here; come back for the normative rule.

This document is the normative reference for the protocol: the envelope,
its semantics, addressing, transactions, errors, discovery, and transports.
The per-command parameter and result schemas are deliberately **not**
duplicated here — they live in the command registry in `crates/api`, which
is their single source of truth, and are published mechanically from it
(§9). This document defines the rules every command obeys; the registry
defines each command.

## 1. Overview

Architecturally the API is a *logical* command bus, not message-bus
infrastructure. There is no broker, no queue server, and no network stack
in the core. The bus is one Rust function — `dispatch(envelope) →
response` — living in a pure crate between the kernel and its hosts.
Transports (§11) are dumb pipes that carry serialized envelopes to that
dispatcher and responses back: an in-process call for the UI and headless
tools, a local socket for the desktop app, stdio for the MCP adapter.

This shape follows from a fact about Hew's document model: **one document
has one owner, and its mutations are strictly ordered.** The kernel
instance holding a document is effectively a single-threaded actor, so
every client must funnel into one serialized dispatch queue regardless of
how the API is packaged. Broker middleware would simulate — with added
dependencies and latency — the single queue the architecture already
provides.

The API is related to, but deliberately distinct from, the kernel's
internal recording and replay machinery (ARCHITECTURE.md §5.5, §5.7). The
internal operation set is private and free to change; the API is a public
contract that maps onto it through `crates/api`, which owns compatibility.
The internal recording format never leaks into the API, and the API's
stability guarantees never constrain kernel internals.

## 2. Design positions

**Commands sit at user-intent altitude.** The API's mutating commands are
the ones a person performs: draw a profile, push/pull a face, boolean two
solids, group, place a component instance. There is no mesh-assembly
surface — no "add vertex", no "add face" — because at tool altitude the
kernel's own construction rules make watertightness a property of the
result *by construction*, and beneath it they cannot. No native modeling
command of this API can produce a leaky solid; the worst it can produce
is a typed refusal — the one honest exception is `hew.doc.import`, which,
exactly as in the UI, yields watertight-or-honestly-leaky Objects from
foreign mesh data. The repair commands (`hew.solid.cap`, `thicken`,
`delete_faces`, `mark_surface`) keep the claim: they take an open object
and either close it into a solid or leave it open, and none of them
opens a solid. This is the load-bearing decision: it is what makes
"an AI agent builds a well-formed, watertight, editable model" a property
of the platform rather than a property of the agent.

**One writer, one queue.** All clients — the UI included — interleave at
whole-envelope granularity on a single dispatch queue. There is no
concurrent mutation model and no locking surface.

**One envelope, one undo step.** Every mutating envelope, whether a single
command or a transaction of many, commits atomically as exactly one entry
in the document's ordinary undo history — the same history the user's own
edits occupy. A user can undo what an agent did, step by step; an agent
can undo what it did itself. There is no second-class edit channel and no
API-private history.

**Typed refusals are answers, not failures.** The kernel already refuses
invalid operations with typed errors carrying plain-language explanations
(ROADMAP.md in this directory, Reliability). The API surfaces those refusals verbatim as
structured, machine-readable results. For an agent, a refusal is the
feedback signal it self-corrects on; for a script, it is a precise exit
condition. A refusal always leaves the document untouched.

**Schema-first, generated everywhere.** Each command is declared once, in
the registry: name, version, parameter schema, result schema, refusal
inventory, documentation string. MCP tool definitions, the TypeScript
client SDK, and the published API reference are all generated from that
declaration. Hand-maintained copies of a command's contract are forbidden
because they will drift.

**The contract ships as tests before it ships as code.** A conformance
suite — golden envelope transcripts, property tests over the transaction
and undo guarantees, registry-drift checks, determinism replay — is
written from this document and the registry declarations and lands
*before* the dispatcher it tests (§14). Implementing a command means
making its pre-written tests pass unmodified; "done" has a mechanical
definition.

**Grants are views, not protocols.** A grant is the part of the command
registry a host lets a connection use (§10). Every grant speaks the
identical envelope; granting more capability never changes the protocol,
only the answer to "which methods may this connection call."

**Transport neutrality.** `crates/api` follows the same purity rule as the
kernel (DEVELOPMENT.md rule 1): no UI, no I/O, no network. Sockets, stdio,
and `postMessage` live in hosts. This is what lets the identical dispatch
semantics serve the live desktop app, the browser build, and a headless
CLI without divergence.

**Determinism makes sessions artifacts.** The kernel guarantees that the
same operation sequence produces bit-identical results (ARCHITECTURE.md
§5.5). An API session logged as envelopes is therefore a reproducible
program: a model built by an agent can be replayed headlessly as a
regression test, byte-for-byte.

## 3. Layering

| Layer | Responsibility | Depends on |
|---|---|---|
| `crates/api` | Command registry, envelope types, dispatch onto the kernel, transaction execution, grant enforcement, schema publication | `kernel`, `inference`, `tessellate` |
| `crates/wasm-api` | Existing WASM boundary; additionally exposes `dispatch` to the host page | `api` (among existing deps) |
| `shells/tauri` | Local socket endpoint (§11.2): accepts connections, authenticates, forwards envelopes to the webview's dispatcher | `app/` |
| `hew-cli` | Standalone binary: MCP adapter, script runner, one-shot dispatch; embeds `api` + kernel directly (headless, the default) or attaches to a running instance (live mode, `--live`) | `api`, importers as needed |

`crates/api` is a kernel-class crate: pure, deterministic, fuzzable, and
testable as ordinary Rust. Its public Rust types *are* the API; JSON is
their encoding at process boundaries, and an in-process caller pays no
serialization at all.

Not every command is answerable by that pure dispatcher: filesystem
paths (`hew.doc.open`, `save`, `export`, `import`), the importer crates,
and rendering (`hew.view.snapshot`) belong to hosts by rule 1. The
registry therefore marks each command **kernel-served** or
**host-implemented**. For the latter, `crates/api` still owns everything
contractual — envelope validation, profile enforcement, parameter
schemas, response shape — and delegates only the effect through a
narrow, typed host trait each host implements to its ability: `hew-cli`
with real file I/O and the importer crates linked, the desktop shell
likewise, the browser build with neither. A host lacking the capability
answers with a typed refusal, advertised via capabilities, never a
protocol error.

## 4. Protocol

### 4.1 Envelope

The wire protocol is **JSON-RPC 2.0**: requests carry `id`, `method`, and
`params`; responses carry the matching `id` and either `result` or
`error`; notifications are requests without an `id` and receive no
response. `method` is the command name; `params` is always a single JSON
object (never positional).

```json
{"jsonrpc": "2.0", "id": 4, "method": "hew.query.entity",
 "params": {"id": "obj_5f3a"}}

{"jsonrpc": "2.0", "id": 4, "result":
 {"kind": "object", "name": "Leg", "watertight": true, "health": "solid",
  "bbox": {"min": [0.07, 0.07, 0.0], "max": [0.13, 0.13, 0.45]}}}
```

Command names are dot-separated, lower `snake_case`, and namespaced
`hew.<area>.<command>` (§7). Parameter and result keys are `snake_case`.
Clients must ignore unknown fields in results and notifications; hosts
reject unknown fields in params (a misspelled parameter is a bug to
surface, not to skip).

JSON-RPC's *batch array* form is not used. Hew's grouping construct is the
transaction (§6), which has semantics — ordering, atomicity, one undo step
— that the unordered JSON-RPC batch cannot express.

Notifications are a server-to-client construct only. A client-originated
notification is invalid at protocol 1: hosts drop it *unexecuted* (a
frame with no `id` can carry no reply, and a mutation that cannot answer
would silently break the refusals-are-answers contract). No mutation
ever rides a fire-and-forget frame.

### 4.2 Handshake

The first request on any connection is `hew.meta.hello`:

```json
{"jsonrpc": "2.0", "id": 0, "method": "hew.meta.hello",
 "params": {"protocol": 1, "token": "…", "client": {"name": "hew-cli", "version": "0.10.0"},
            "encodings": ["json"]}}
```

The response reports the host's protocol version, application identity,
the granted profile (`core` or `app`, or `plugin` with a `grant` member
for a restricted connection, §10), the negotiated encoding, and the open
documents:

```json
{"jsonrpc": "2.0", "id": 0, "result":
 {"protocol": 1, "app": {"name": "hew", "version": "1.0.0"},
  "profile": "app", "encoding": "json",
  "documents": [{"id": "doc_1", "title": "Bird feeder", "path": "…"}]}}
```

Any request before a successful `hello` is rejected with `-32004` (not
ready). A host that cannot serve the client's requested protocol version
answers `hello` itself with an error naming the versions it speaks.

On transports where a host serves multiple documents (the desktop app
with several windows), a connection is bound at connect time to the
document that was front-most then — requests to it still answer `-32002`
until the connection attaches — and `hew.doc.attach {"document": ...}`
attaches (or rebinds) it. `document` is a document id (`doc_N`), or a document's exact
`path`, `label`, or `title` when that is unambiguous; an unmatched or
ambiguous reference is refused (`unknown_document`, `ambiguous_document`,
the open documents in `detail`) rather than guessed at, so a client can
never go on to edit a window it did not mean to. The result carries the
document's entry. Without `document` it attaches to the connection's
current one. Every subsequent envelope on that connection addresses the
attached document; a connection is attached to at most one document at a
time. Document ids are unique for the host's lifetime and never reused,
so a stale attach can never silently target the wrong document; closing a
document detaches every attached connection, whose subsequent requests
answer `-32002`. In hosts that honor `hew.doc.new`/`open`, the issuing
connection is attached to the resulting document automatically — no
separate `attach` round-trip.

`hew.meta.documents` (and `hello`'s `documents`) list the open documents,
oldest window first. Each entry is `{id, title, path, dirty, document_id}`
— `path` is `null` until the document has a file, `dirty` is unsaved
changes, `document_id` is the persistent UUID the document's file carries
(lowercase, hyphenated; `null` until the document is first saved, see
HEW_FILE_FORMAT.md §2) — and a multi-window host adds `front` (the user's front-most window), `attached`
(this connection's current document), and `label` (the name given at
`hew.doc.new`/`open`, if any). It is host-wide: it needs no attached
document and works however the connection is bound.

Addressing is also available **per request**: any envelope may carry a
top-level `"document"` member naming a document, and that one request runs
there. The connection's attachment does not change, and the request
needs no prior attach. Where a host has one document (headless), the
member is accepted only if it names that document (`doc_1`). A restricted
connection (a plugin or a connected app, §10) without `documents.other`
may not use the member at all: it is refused `-32001`
`document_not_granted` whatever document it names. Reaching a
background document this way — or by attaching to it — never raises it:
nothing in the protocol moves the user's window order except the explicit
`hew.doc.focus`, and `hew.doc.new`/`open` place their new window behind
the user's current one unless `focus: "front"` says otherwise.

The desktop host's window commands, in the `app` profile only except
`new`/`open`:

- `hew.doc.new {focus?, label?}` — a fresh document in a NEW window
  (never replacing one), attached to the connection; headless hosts
  replace the working document. Result `{document}`.
- `hew.doc.open {path, focus?, label?}` — a `.hew` file in a new window,
  or, if some window already has that file open, an attach to that window
  (`already_open: true`) — two windows are never two writers of one file.
  Result `{document, already_open?}`.
- `hew.doc.close {document?, discard?}` — close a window. A document with
  unsaved changes refuses `unsaved_changes` unless `discard` is true; the
  last open document refuses `last_document` (a live connection needs
  something to attach to). A window whose page has hung (it does not
  answer within 30 seconds) closes anyway when `discard` is true — the
  shell destroys it — since there is no other way to close it.
- `hew.doc.focus {document?}` — raise a window to the front. The one
  command whose purpose is to move the user's focus.

### 4.3 Encoding

JSON (UTF-8) is the canonical and, at protocol 1, the only encoding. This
is a deliberate consequence of command altitude: commands and results are
small — the heavy payloads (tessellation buffers) never cross the public
API. The `encodings` field of the handshake exists so that a future host
(most plausibly the out-of-process kernel of ARCHITECTURE.md §3.2) can
negotiate a binary encoding without an envelope change. No binary encoding
is defined today.

Lengths are `f64` meters, angles are radians, coordinates are
right-handed with +Z up, in the world frame — always, on every command,
regardless of the document's display units or movable drawing axes.
Display formatting is a UI concern the API does not model. Non-finite
numbers (NaN, ±Inf) are invalid anywhere in an envelope.

### 4.4 Errors

Three classes, distinguished by JSON-RPC error code:

- **Protocol errors** — malformed JSON, unknown method, invalid params,
  method outside the granted profile, no attached document. Standard
  JSON-RPC codes (`-32700`, `-32601`, `-32602`) plus `-32001` (not
  permitted in profile), `-32002` (no document attached), and `-32004`
  (no successful `hello` yet). `-32601` is reserved for methods the
  registry has never heard of; a real command the connection was simply
  not granted is always `-32001`.
- **Refusals** — code `-32000`. The command was well-formed and the kernel
  declined it, document untouched. `error.data` carries the typed refusal:

  ```json
  {"jsonrpc": "2.0", "id": 9, "error":
   {"code": -32000, "message": "refused",
    "data": {"refusal": "push_pull_obstructed",
             "failed_index": 0, "failed_method": "hew.solid.push_pull",
             "detail": {"object": "obj_5f3a", "distance": -0.3},
             "explanation": "Pushing this face 30 cm would drive it through
                             geometry behind it. Try a smaller distance…"}}}
  ```

  This `error.data` shape is canonical for every refusal: `refusal`,
  `failed_index`, `failed_method`, `detail`, and `explanation` are always
  present, whether the envelope was a transaction (the index of the
  refusing command) or a plain request (index `0`) — the two framings of
  the same command produce the same shape. `detail` is refusal-specific
  structured data, its schema named per refusal in the registry's
  inventory. `refusal` is a stable machine name drawn from the kernel's
  typed error
  inventory; `explanation` is the same plain-language text the UI shows
  (already exhaustively maintained — ROADMAP.md in this directory, Reliability). New refusal
  names may appear in any release; clients must treat an unrecognized
  refusal as "refused, document untouched" and read the explanation.
- **Internal faults** — code `-32003`. A kernel invariant failed (the
  always-on validator caught a would-be corruption and rolled back). The
  document is untouched; the condition is a bug to report, not to handle.

### 4.5 Notifications and events

A host sends **notifications**: frames with no `id`, whose `method` is
`hew.event.<name>`, and which answer nothing. They share the envelope,
so no transport needs a second channel; on a socket they arrive between
replies, never inside one, and an event a request caused arrives after
that request's reply. A client sets any frame without an `id` aside as
an event rather than taking it for a reply. §4.1's rule still
holds the other way: a notification a client sends is dropped
unexecuted.

```json
{"jsonrpc": "2.0", "method": "hew.event.document_changed",
 "params": {"document": "doc_1", "generation": 2, "epoch": 2,
            "origin": {"connection": "live:0"}, "label": "Profile",
            "kind": "commit", "bookkeeping": false,
            "changes": {"added": ["skt_0"], "removed": [], "modified": [],
                        "attrs": [], "document": false, "full": false}}}
```

**Subscribing.** A connection receives nothing until it subscribes.
`hew.event.subscribe {"events": [<name>, ...]}` (1 to 32 names) needs a
successful `hello` but no attached document, and every grant may call it.
It subscribes each name the grant may receive and lists the rest in
`denied` (denied, not refused); an unknown name is `-32602` `unknown
event name: <name>` and changes nothing. The result is `{"subscribed":
[...], "denied": [...]}`, `subscribed` being every name the connection
is subscribed to afterwards, in name order. `hew.event.unsubscribe` takes
the same params and answers the same shape (`denied` empty). The grant
is checked again whenever a frame goes out, so a narrowed grant takes
effect without a reconnect.

| Event | Needs | Params | Sent when |
|---|---|---|---|
| `document_changed` | `model.read` | below | after every history entry, undo, redo, aborted transaction or plugin command, and document replacement |
| `context_changed` | `model.read` | `{document, stack, fallback}`, both as `hew.query.context` answers them | the session stack, or the app's in-context entries, changed |
| `scenes_changed` | `model.read` | `{document, scenes, active}`: `hew.scenes.list`'s entries, and the last activated Scene's id or `null` | a Scene was added, removed, renamed, reordered, updated, or activated |
| `units_changed` | `model.read` | `{document, units}`, a display length format (`m`, `cm`, `mm`, `arch`, `frac_in`, `dec_in`) | the app's display units changed |
| `document_will_save` | `model.read` | `{document, title, format}` | before a save |
| `document_saved` | `model.read` | `{document, title, path?, format, document_id}` | after a save landed |
| `document_opened` | `model.read` | `{document, title, path?, document_id}` | a document was opened into a window |
| `document_closed` | `model.read` | `{document, title, path?}` | a document was closed or replaced in a window |
| `selection_changed` | `view` | `{document, ids, primary, generation, epoch}`: the selection as `hew.selection.get` answers it (§7.3) | the selection changed, in the app or over the bus |
| `tool_changed` | `view` | `{tool}`, the tool's name as the toolbar names it (`"Select"`, `"Line"`, ...) | the active tool changed |
| `plugin_state` | nothing | `{plugin, state, detail?}` | a plugin loaded, unloaded, crashed, or its grant changed |
| `slot_changed`, `settings_changed`, `ui_cancel`, `window_closed`, `surface_closed` | nothing | per their surfaces | plugin surfaces |

`plugin_state` and the surface events can be subscribed to today and
arrive once the features that send them exist. `selection_changed` and
`tool_changed` are sent by the desktop app (§7.3). The document lifecycle events go only to a connection bound
to the event's document, unless its grant holds `documents.other`, and
carry `path` only under `documents.save` (first-party connections hold
both). On the desktop a first-party socket connection hears these events
for every window, not only the one it is attached to: the app offers each
window's lifecycle frame to every connection subscribed to it that holds
no session in that window (a window nobody is attached to still reports
`document_opened`). `document_closed` is sent as the page goes away and
can be lost when the window is destroyed before the frame is written.
Every event names its `document`: the window's id on the desktop
(`doc_N`), `doc_1` elsewhere. `document_will_save` waits for nobody.

**`document_changed`.** `generation` and `epoch` are the document's
clock right after the change (§7: `hew.history.status`). `origin` is
`"user"` or `{"connection": "<identity>"}`, as `hew.history.status`
reports it, and `label` is the entry's label as the history lists it
(`""` for an abort or a replacement). `kind` is `commit` (an entry
pushed, or a `coalesce` commit extending one), `undo`, `redo`, `abort`
(a transaction refused partway, or a plugin command aborted: the
document went back to how it was when the bracket began), or `reset`
(the document was replaced: opened, created, reverted). `bookkeeping`
marks entering or leaving a group or component edit, which carries no
changes.

`changes` names what the change touched, by public id: `added` (created
or placed), `removed` (deleted, consumed by a boolean or a sweep,
exploded, ungrouped), `modified` (everything else it touched, groups
whose membership changed included), and `attrs` (`{id, ns, scope}`
entries whose attribute dictionaries changed: `scope` `"entity"` with
`id` the entity, or `"document"` for the document's own; `scope`
`"face"` or `"edge"` when a solid face's or edge's dictionary changed,
with `id` the owning object, §8). An undo's lists are already the undo's: `added` names
what it brought back, `removed` what it took away. `document: true`
means document-level state changed (the drawing axes). `full: true`
means the lists are empty and anything may have changed: an import, a
whole-model rescale, an abort, a reset; a subscriber re-reads what it
cares about. A transaction or a plugin command is one event however
many commands it ran: while a plugin command runs, the envelopes it
commits produce no events, and when it ends its one entry does (or, if
it is aborted, one `abort` and nothing of what it did).

The host emits one `document_changed` per change since it last sent
events. More than 64 at once, or a gap its history no longer reaches,
collapse into one `commit` with `full: true` at the current clock.

**Catching up.** `hew.query.changes_since {"generation": n}` answers
`{"generation", "epoch", "from": n, "changes"}`: the union of every
change since the client saw generation `n`, undo lists swapped as above,
`full` sticky. A generation the session's history no longer covers (the
host keeps the last 256 changes), or never had, answers `full: true`. An
abort rewinds the generation, so after one a generation can name two
states; a client that saw the abort's `document_changed` re-reads.

**Transports.** On the desktop's Unix socket (macOS, Linux) and its
named pipe (Windows) each connection has one writer that sends replies
and events alike, whole lines in order; the pipe is overlapped, so a
read waiting on the client's next frame never holds up a reply or an
event. A client that stops reading, so that a write to it waits 10
seconds, has its connection closed on either transport. `hew-cli` keeps
the events it reads while waiting for a reply (§12). An in-page plugin
receives its frames on its port.

## 5. Identity and addressing

### 5.1 Entity identifiers

Objects, Sketches, Groups, Component definitions, Instances, Guides,
Materials, Tags, and Annotations (dimensions and leader texts, `ann_…`)
are addressed by opaque string identifiers (`"obj_5f3a"` above is
illustrative; clients must not parse ids). `hew.query.describe` answers
an annotation id with its kind, anchors, offset and text;
`hew.entity.delete` removes one.

Neither of the kernel's own identity mechanisms is stable enough to hand
to a client: runtime handles are generational (deleting and re-creating
an element changes them — DEVELOPMENT.md §4), and the `.hew` format's
dense ids are renumbered per save (HEW_FILE_FORMAT.md forbids caching
one across loads). The public id space is therefore an **api-owned
indirection**: `crates/api` assigns each entity a stable public id and
maintains the mapping to live kernel handles, including across undo and
redo — deleting an entity retires its id, and undoing the deletion
restores the same id, whatever handle the kernel reallocates underneath.

Making that stability survive save and load requires a persistent
per-entity identifier in the `.hew` manifest. This spec calls for it as
a deliberate 1.0 file-format addition traveling with §8's attribute
dictionaries, under the same rule-8 discussion and the same-commit
HEW_FILE_FORMAT.md obligation; with it, public ids are stable across
save and load, which is what `hew-cli dispatch --file` sequences and
attribute-dictionary consumers need. Sketch sub-entities — regions,
islands, and curves — surface through the same api-owned indirection, as
compound public ids that embed their owning sketch: the kernel's raw
sub-entity keys are sketch-local, meaningless on their own, so a public
id must fully name the entity by itself — which is what lets
`hew.solid.extrude` and `follow_me` take just a region id where the
kernel's own entry points take a sketch and a region separately. They
keep the derived, identity-stable semantics the kernel maintains for
them (ARCHITECTURE.md §2.6), stable within an open document session;
clients re-resolve them after open or attach.

**Scenes are not entities.** A Scene (§7.1) carries a kernel stable id
minted from the same shared counter as every entity's, but it is not an
`EntityRef`: it never appears in `hew.query.scene`'s tree, carries no
attribute dictionaries (§8), and is addressed by its own list
(`hew.scenes.list`), not by locators. Its public id is therefore its own
top-level shape, `"scene_<hex>"` — the hex of its stable id, exactly like
`public_id`'s entity prefixes (`obj_`, `skt_`, …) but minted by a
dedicated helper rather than that function, since there is no
`EntityRef::Scene` to hand it. It parses back to the stable id the same
way: opaque to clients, not to be constructed or decoded by hand.

### 5.2 Faces and edges: solid geometry by locator, sketch edges by id

Faces and edges of a **solid** deliberately have **no persistent public
identifiers**. Under sticky geometry a face that is split, merged, or
consumed is not "the same face" in any way the kernel could honestly
promise across edits, and a heroic face-identity scheme would be a
permanent tax on every kernel operation. Commands that take a solid face
or edge take a **locator** instead — the API's equivalent of pointing:

- **By point** — `{"object": "obj_5f3a", "at": [0.1, 0.1, 0.45]}`: the
  face (or edge) of that object containing, or nearest within tolerance
  to, the point. Ambiguity (a point on a shared edge of two candidate
  faces, when a face was required) is refused typed, never guessed.
- **By ray** — `{"object": "obj_5f3a", "ray": {"origin": …, "dir": …}}`:
  first hit on the object, the programmatic form of clicking.
  (`hew.query.raycast` returns a by-point locator for what it hit: `at` is
  in the hit Object's own frame, §7.4.)
- **By transaction handle** — `{"$face": "cut#top"}`: a face token
  returned by an earlier command *in the same transaction* (§6.2); the
  key after `#` names one of the faces that command's registry entry
  documents (§5.4). Tokens are transaction-scoped and expire with it;
  they are ergonomics, not identity.

Additional predicate locators (e.g. "the face with normal +Z of greatest
area") are *Reserved*: the shapes above are sufficient for the first
release, and predicates can be added additively.

**Sketch edges are the opposite case, and deliberately so.** A sketch's
edges are not sticky-geometry byproduct the way a solid's faces are —
they are the user-authored scaffolding itself: exactly the lines a
person drew, that a person also trims, extends, and re-draws around,
often long after the fact (§1's "you'll use constantly" pairing of
drawing and erasing). A sketch has no analogue of face split/merge/
consume silently retargeting what an id would have named — an edge
exists exactly until something explicitly deletes it — so the
"heroic identity scheme" cost above never applies, and the api-owned
compound-id treatment already given to a sketch's regions and curve
chains (§5.1) extends to its edges too: `"edg_<sketch>_<key>"`, minted
and resolved exactly like a region or curve id. `hew.query.entity` on a
sketch lists its edges (id, endpoints, owning curve if any) alongside
its regions (each with `face`: `false` for a loop of edges with no face,
which `hew.solid.extrude` refuses `region_has_no_face`) and curves, and
answers a query on an edge id directly;
`hew.entity.delete` accepts one to erase exactly that edge — the
eraser's own tool, one edge, one undo step — leaving whatever the
kernel's own sticky rules do to the vertices and regions it touches
(never repaired or second-guessed at the API layer — DEVELOPMENT.md
rule 4).

A client that has not queried a sketch yet can still name one of its
edges the way a person clicks it, with the same locator shapes a solid
edge uses, under a `"sketch"` key instead of `"object"`:

- **By point** — `{"sketch": "skt_2a1", "at": [0.5, 0.0, 0.0]}`: the
  sketch's edge nearest the point, within tolerance; ambiguity refuses
  typed exactly like the solid-edge form above.
- **By endpoints** — `{"sketch": "skt_2a1", "from": [0, 0, 0], "to": [1,
  0, 0]}`: the edge whose two endpoints coincide with the given points
  (either order). Sticky rules forbid coincident duplicate edges, so
  this match is unique by construction — no ambiguity case exists for
  it.

Wherever this spec says "edge locator" (here and in §5.3), all three
forms are legal: a solid's `{object, at|ray}`, a sketch edge's own
public id as a bare string, or a sketch edge's `{sketch, at|from+to}`.

**Occurrence paths.** A component definition's members have one public
id each, but they are placed once per instance, so an id alone cannot say
which placement is meant. An occurrence locator names one:
`{"path": ["ins_a", "ins_b", "obj_c"]}` lists the instances from the world
down, then the node itself. The first instance is a world instance; each
later element is a member (directly or inside the definition's groups) of
the previous instance's definition. A one-element path is a world node,
the same placement its bare id names. `hew.query.occurrences` lists every
current path (its rows carry `path` without the node, and `id`, so
`{"path": [...row.path, row.id]}` locates a row). A path that leads
nowhere refuses `unknown_occurrence`; a bare id naming a definition
member refuses it too, since only a path says which placement is meant.
Paths are stable within a session while the instances on them stay
live. They are not promised across a save and reload: list again after
one.

### 5.3 Derived points

A UI user rarely types coordinates — the inference engine's magnetic
points (endpoint, midpoint, center, quadrant) supply exactness for them.
The API gives clients the same magnetism symbolically: anywhere a command
accepts a 3D point, the client may pass either coordinates `[x, y, z]` or
a **derived-point locator** naming a point of existing geometry:

- `{"point": "midpoint", "of": {"edge": <edge locator>}}`
- `{"point": "endpoint", "of": {"edge": <edge locator>}, "nearest": [x, y, z]}`
  — an edge has two endpoints; `nearest` picks one
  (`<edge locator>` here is §5.2's full grammar — a solid edge's
  `{object, at|ray}`, or a sketch edge named by its own public id or
  its `{sketch, at|from+to}` locator: the midpoint of a rectangle a
  client just drew is exactly as reachable as the midpoint of a
  push/pull'd solid wall)
- `{"point": "center", "of": <curve id | face locator>}` — the analytic
  center of a drawn circle or arc, or of a stamped curved wall's cylinder
- `{"point": "centroid", "of": <face locator>}` — a planar face's area
  centroid
- `{"point": "quadrant", "of": <curve id>, "toward": [x, y, z]}` — the
  quadrant point of a circle or arc in the given direction
- `{"point": "bbox", "of": <entity id>, "anchor": "center" | "min" | "max"}`
- `{"point": "position", "of": <guide id>}` — a guide *point*'s location

Guides are addressable entities but not box-shaped: a guide *line* is an
origin and a direction, infinite for one Hew draws and a ray or a segment
where a `.skp` file holds one, so `bbox` anchors on one refuse typed
rather than fabricating a meaningless corner. Its origin, direction,
stretch (`from` and `to`, meters along the direction from the origin,
`null` for an end that runs on forever) and `tags` (paths, as a node's; a
guide with a hidden tag is in the document but neither drawn nor snapped
to) read back via `hew.query.entity`, and intersection-based alignment (the crossing of two guides) is client-side
arithmetic at 1.0; the Reserved snap-resolution form below is the future
shortcut.

A derived point resolves to exact `f64` coordinates **at dispatch time,
against the document state the command actually runs in** — inside a
transaction, that means after the preceding commands have applied. This
is why the symbolic form is the intended idiom rather than
query-then-paste-coordinates (which remains perfectly legal): resolution
is exact by construction — the kernel derives the point from live
geometry, with no rounding through a transcript — and it cannot go stale
between a query and the mutation that uses it. A locator that fails to
resolve — dangling entity, no such point, ambiguous result — refuses
typed and aborts its transaction, document untouched.

`hew.query.resolve` resolves any locator (point, face, or edge — §5.2's
full edge grammar, solid or sketch) to its concrete value without
mutating, for inspection and debugging; a resolved edge's `"kind"` field
(`"solid"` or `"sketch"`) says which grammar matched. Proximity-based
snap resolution — "the strongest snap of these kinds near this point",
the full inference engine as a service — is *Reserved*; the derived
forms above are deterministic and cover the first release.

### 5.4 Creation results

Every command that creates an entity returns its id. Commands that
create or reshape solids additionally return transaction-scoped face
tokens (§5.2), and *which* faces earn tokens is not left to
interpretation: each command's registry entry names its token keys
normatively. `hew.solid.extrude`, for instance, returns `base`, `top`,
and `side.<n>` in boundary-loop order; a through-cutting `push_pull`
names the wall faces it opened. A token is addressed
`{"$face": "<label>#<key>"}`. A client never has to re-query to learn
what it just made.

## 6. Transactions

### 6.1 One envelope, atomic, one undo step

A transaction is a single request:

```json
{"jsonrpc": "2.0", "id": 7, "method": "hew.doc.transact",
 "params": {"label": "Table leg",
  "commands": [
   {"method": "hew.sketch.draw_circle", "as": "profile",
    "params": {"plane": {"ground": true}, "center": [0.1, 0.1, 0.0], "radius": 0.03}},
   {"method": "hew.solid.extrude", "as": "leg",
    "params": {"region": {"$ref": "profile#/region_id"}, "distance": 0.45}},
   {"method": "hew.entity.rename",
    "params": {"id": {"$ref": "leg#/object_id"}, "name": "Leg"}}]}}
```

The commands execute in order against the attached document, atomically:
either every command commits and the transaction becomes **one undo
entry** (labeled `label`), or the first refusal aborts the whole
transaction with the document exactly as it was, reporting the failing
index and the refusal:

```json
{"error": {"code": -32000, "message": "refused",
 "data": {"failed_index": 1, "failed_method": "hew.solid.extrude",
          "refusal": "…", "detail": {…}, "explanation": "…"}}}
```

The successful result carries the per-command results in order:
`{"results": [ … ], "label": "Table leg"}`.

The one-entry guarantee is a real kernel obligation, not api-layer
bookkeeping: it requires a labeled **compound history entry** — a
history node grouping the transaction's recorded ops, undone and redone
atomically by replaying its members (inverses in LIFO order) under the
same proof-carrying replay contract as any single op (DEVELOPMENT.md
rule 9). That kernel extension — together with the per-entry **label**
and **origin** it implies, which `hew.history.status` reports (origin is
`user` for UI-authored edits and the connection's identity for API
envelopes; both are session-scoped state, never serialized into `.hew`)
— is a rule-8 item and a prerequisite: it lands before the dispatcher
does. The kernel provides it as the `begin_transaction` /
`commit_transaction` / `abort_transaction` bracket on `Document`, whose
committed compound entry carries exactly this label and origin.
ARCHITECTURE.md §2.11's per-operation undo granularity then describes
UI-authored edits; an API envelope is one atom in the same shared
history.

A model-mutating method (§6.4) sent as a plain request outside
`transact` is exactly equivalent to a one-command transaction (its label
defaults from the command). Transactions do not nest, and there are
deliberately **no
open-ended transactions across envelopes**: a held-open transaction would
either block the user's live editing or interleave into it, and both are
unacceptable. Clients plan, then submit.

**Compounds.** The one exception is a forwarded plugin command (§10.2)
that can change the document: a plugin command is a critical section on
the document, and the kernel brackets it. The host opens a **compound**
around the forward before the plugin runs: every envelope the plugin
issues while serving it commits as a child of that compound, and when
the command ends the children become **one undo entry**, labeled with
the command's declared label (or its name) and attributed to the
provider plugin (origin `plugin:<id>`). A plugin command that calls
another plugin's command nests: the inner compound lands as one child of
the outer. A refusal, a timeout, or the user's Cancel **aborts** the
compound, which restores the whole document to what it was before the
command began: model, both history stacks, the saved mark, and view
state alike. While a compound is open nothing else commits: every other
client's frame waits, in arrival order, until it ends (the desktop app
queues it, §11.2), and the document-replacing commands (`hew.doc.open`,
`new`, `revert`, `close`) are refused `-32000` `document_busy` from any
connection that does reach the dispatcher, because an abort cannot undo
them. docs/agents/PLUGINS.md "Compounds" is the plugin-side account.

### 6.2 References between commands

A command may be labeled with `"as"`. Any parameter value in a later
command of the same transaction may be the object
`{"$ref": "<label>#<json-pointer>"}`, which is replaced by the value at
that JSON Pointer in the labeled command's result before dispatch — and
`{"$face": "<label>#<key>"}` for face tokens (§5.2).

Failure splits by when it is knowable. Statically checkable defects — a
reference to an unknown label, a *forward* reference to a later command,
malformed pointer syntax, a `$ref` or `$face` outside a transaction —
are protocol errors for the whole envelope (`-32602`), caught before any
command runs. A syntactically valid pointer that fails to resolve
against the result it names (the command legitimately produced no
`region_id`, say — result shapes may vary with the geometry) is knowable
only at execution time: it aborts the transaction atomically, document
untouched, reported in the canonical §4.4 shape with the failing
command's index and the machine name `ref_resolution_failed`.

### 6.3 Editing context

The API exposes the same enter/exit editing-context model the UI has
(ARCHITECTURE.md §2.5, §2.11): `hew.context.enter {id}` opens a group or
component session frame, `hew.context.exit` closes the innermost, and
drawing commands obey the current context's sticky rules. The context
stack is shared document state — one stack, seen by every client and by
the user.

Because a dangling frame left by a remote client would silently change
what the user's next stroke welds to, context commands are legal **only
inside a transaction**, and a transaction must be **context-balanced**:
every `enter` it performs is `exit`ed within the same envelope, and an
`exit` may only close a frame opened by the same envelope — never a
frame the user, or a prior envelope, already had open. A bare
`hew.context.enter` or `exit` request, and any unbalanced or
over-exiting transaction, is rejected as a protocol error (`-32602`)
before anything runs. Group entry has its own eligibility gate: a
session opens on a *top-level* group only — the kernel refuses a
directly-addressed nested group typed (that refusal exists precisely as
the backstop for API-shaped callers skipping levels) — so a nested
container is entered outermost-first, one balanced `enter` per level,
exactly as the UI drills down. Since the shared stack still changes what drawing
commands weld to, `hew.query.context` reports the current stack, so a
client can plan against — or decline to fight — whatever frame the user
has open.

The eligibility gates on entering a component instance (similarity pose,
no elsewhere-grouped sibling placement) surface as typed refusals at
protocol 1. This is deliberately narrower than the UI, which falls back
to the in-context editing model for such instances (ARCHITECTURE.md
§2.11): the fallback machinery operates through the pose without baking,
a mode the API does not model yet. The gap is registry-documented; an
agent's recourse is `make_unique` or `explode`, and fallback-mode
editing can arrive additively later (§9).

Moving, turning, scaling, copying, or cutting a definition member does
not need a session at all: the path forms of §7 edit the definition
directly, in world terms, with every placement following.

**The user's own context.** `hew.view.enter`, `hew.view.exit`, and
`hew.view.exit_all` (§7.3) are the other side of this: they act on the
person's window outside any transaction, the way their click would. They
are not `hew.context.enter` and `exit`: those stay balanced and scope a
plugin's own edits; these leave the window inside the node when they
return, and the frame they open is the person's, visible in the
breadcrumb and closed the way the person closes frames. `hew.view.enter`
takes an `id` (an object, group, or instance in the world) or a `path` (an
occurrence path of instances outermost first; the last element is the
node to enter and may be a member object inside the last instance), opens
group and component sessions level by level, falls back to the in-context
entry for an instance a session cannot take (unevenly scaled, or placed in
a group elsewhere; a mirrored instance opens a session like any other),
and selects the target. `hew.view.exit` pops the innermost entry, a
fallback entry or a session frame; `hew.view.exit_all`
goes back to the top level. Each answers `{}`: the outcome is the
`context_changed` event, which carries the new `stack` and the `fallback`
entries (`{kind: "object" | "instance", id}`, outermost first, inside the
innermost frame). A refusal the app shows the person as a toast leaves the
context unchanged and sends no event; `hew.query.context` afterward
reports the truth. While a tool gesture is armed in the window (a Move
drag in flight) all three refuse `gesture_in_progress`. A call inside a
multi-command transaction is `-32602`. Each is `mutates_document: true`,
because opening or closing a session is a history step the window
resyncs after, but none is a replayable kernel call. Only closing a
session pushes an undo entry: an exit that pops just a fallback entry
changes the window's context and nothing in the history.

### 6.4 What a transaction may contain

The registry classes every command, and the class governs placement:

- **Model-mutating** commands — sketching, solids, transforms,
  structure, materials, tags, guides, attribute writes, context — are the
  transaction's payload, and the only class the one-envelope-one-undo
  accounting applies to.

  Three of these are **registry-state**: `hew.material.create`,
  `hew.tag.create`, and `hew.tag.set_visible` follow model-mutating
  placement rules (transaction payload allowed) but record no undo entry.
  The kernel deliberately keeps palette and registry additions outside the
  undo log — an unreferenced material is harmless, and a tag's visibility
  is view state, not a modeled edit. Their registry entries note this in
  their summaries. `hew.library.insert` of a `material` item is the same
  kind of write — it only adds palette entries — and also records none.

  `hew.tag.rename` is an ordinary undoable edit: it moves a tag path — and
  every tag nested under it — to a new path while keeping the tag's
  identity (stable id, hidden flag, attributes; a Scene's captured hidden
  tags follow it) and rewriting every node that carried it. It refuses an
  unknown source (`unknown_tag`), a target already in use
  (`duplicate_tag`; tags are never merged), and an empty or self-nested
  target (`invalid_tag_path`).
- **Read-only** commands — `hew.query.*`, `hew.compute.*`, `hew.meta.*`,
  and `hew.attr.get` — are legal anywhere: standalone (adding no undo
  entry), or interleaved inside a transaction, where their results are
  legitimate `$ref` sources (measure mid-plan, use the number).
- **Solitary** commands — document lifecycle (`hew.doc.new`, `open`,
  `attach`, `save`, `export`, `import`), `hew.history.*`, and
  `hew.view.snapshot` — are legal only as the sole command of their
  envelope: they are host effects or history operations, not recorded
  model mutations a compound entry could roll back. They remain
  reachable through a one-command `hew_transact` (§13), which is their
  canonical invocation from MCP; `hew.doc.import` commits as one undo
  entry of its own.

A transaction containing a solitary command alongside anything else is
rejected statically (`-32602`, naming the offending index).

A plugin-provided command (§10.2) is never transactable, whatever its
class, and its capabilities entry says so (`"transactable": false`). A
`hew.doc.transact` whose sole command is a plugin command unwraps to that
command's forward and is answered in the transaction shape (`{results:
[result], label}`, the label being the envelope's `label`, else the
command's declared label, else its name); one holding a plugin command
beside anything else, or two plugin commands, is refused `-32000`
`plugin_command_in_transaction` with the plugin command's index, before
anything runs.

### 6.5 Coalescing and compare-and-swap

Two optional members of `hew.doc.transact` let a client that edits in
small steps keep the history readable and never write on a stale model.

**`coalesce`** (a string of 1 to 64 bytes; anything else is `-32602`
`coalesce must be 1..=64 bytes`). A transaction that commits under a key
extends the top undo entry instead of pushing a new one when that entry
was committed under the same key by the same origin, the connection
identity history records (§6.1): ten field edits in a parts list are one
undo step, labeled with the first edit's label. Connections that share
an identity (two sessions of one plugin, say) share their keys.
The history generation still moves on every such commit. A key expires
as soon as the history generation moves without it: entries never
coalesce after any other entry lands, after any undo or redo (even when
the same entry is on top again), across a different origin or a
different key, or for a commit with no key. A `hew.doc.transact` whose
sole command is a plugin command refuses `coalesce` (`-32602`): that
command's undo entry is its own compound, which no key can extend. Nor across a save: when the
document was saved with the top entry in place, the next keyed commit
lands as its own entry, so the edit after a save leaves the document
unsaved and undoing just that edit makes it clean again.

**`expect_generation`** (a non-negative integer). When the document's
history generation is not this value as the envelope is dispatched, the
envelope is refused `-32000` `generation_moved`, with `detail: {expected,
actual}`, before any command runs (the static checks of §6.2 to §6.4
included), attributed to `hew.doc.transact` at index 0. It applies to a
sole plugin command too: the forward is not issued. The generation
is what `hew.query.*` and `hew.history.status` report as `generation`
(§7): a client that reads, computes, and commits with the generation it
read never overwrites a change it did not see. It counts every entry
pushed (each command's own and the transaction's), every undo and redo,
and every coalesced commit. This is the default write pattern for
plugins (docs/agents/PLUGINS.md invariant 13).

### 6.6 Rehearsal (`dry_run`)

**`dry_run: true`** on `hew.doc.transact` rehearses the envelope: it runs
on a copy of the document and answers exactly what a commit would answer,
then the copy is dropped. Nothing changes: no undo entry, no history
generation or epoch move, no event, no `document_changed`, no
recording, no selection or view effect. The answer is the commit's
`{results, label}` with `dry_run: true` beside them; the results are
byte for byte the ones the same envelope committed now would return, the
ids a commit would mint included, so a plugin can rehearse an edit and
read the rehearsed geometry back in the same call (a `hew.query.mesh`
after the edit, through `$ref`). A refusal is the commit's refusal, same
`failed_index`, same name.

- **Checks.** The envelope checks come first, as for a commit and
  before the attachment check: a rehearsal against a stale read refuses
  `generation_moved`, a `coalesce` outside 1..=64 bytes is `-32602`, and
  only then does an unattached connection get `-32002`. A `coalesce` of
  any length is `-32602` ("coalesce does not apply to a dry run; nothing
  is committed"). A document-busy host refuses a dry run as it refuses
  any frame.
- **Reads.** What a command reads from its host (the app's in-context
  entries and armed gesture for `hew.query.context`, the selection, the
  isolation, the open documents) is the live host's answer, as for a
  commit; only the library stays out of reach. Every write to the host
  is refused (below).
- **The host is told.** Every answer to a dry run is one, refusals made
  before the rehearsal ran included (`compute_busy`, `document_busy`, a
  permission or attachment check): the desktop app tags it and skips
  what it does after an edit (recording, the viewport refresh, the
  selection write-back), and `hew-cli dispatch --file` does not save.
- **Permission.** A dry run needs read permissions only: a command whose
  permission is `model.write` or `attributes.write` (any namespace)
  counts as `model.read`: a plugin allowed only to read may rehearse an
  edit it may not commit, which leaks nothing. The attribute namespace rule
  (§8) still runs inside the rehearsal, so a rehearsed write under a
  namespace the grant does not own refuses `attr_namespace_not_granted`
  as a commit would.
- **What cannot be rehearsed.** A Host-served command (files, the
  library, the screen, the selection), a plugin command, and a view
  effect a command would ask of the host (`hew.view.hide`, a Scene's
  apply) refuse `-32000` `dry_run_host_command` with `detail: { method }`
  and the command's index. A solitary Kernel-served command
  (`hew.history.undo`, `hew.doc.attach`, …) is `-32602`.
- **Cost.** One copy of the whole document per call. That is right for a
  check before a commit and wrong for a preview on every pointer move:
  previews use `hew.compute.*` (§7.6), which copy only their operands.

## 7. Command surface

The namespaces below and the semantics stated for them are normative; the
per-command schemas are the registry's (§9). Tiers: **Required** commands
exist at 1.0 and are the minimum for the agent-modeling vision; **Standard**
commands exist at 1.0 for tool parity; **Reserved** namespaces are designed
for, not shipped.

| Namespace | Contents | Tier |
|---|---|---|
| `hew.meta` | `hello`, `capabilities`, `documents` (the open documents: id, title, path, dirty, document_id; in the desktop app also front, attached, label — §4.2) | Required |
| `hew.doc` | `attach` (to a named document, §4.2), `transact`, `new`, `open` (the desktop app opens each into a new background window), `close`, `focus` (desktop app windows; `app` profile), `save`, `export` (STL/3MF/glTF/USDZ), `import` (foreign formats), `purge_unused` (delete every unused palette material and component definition, one undo entry), `health` (every open object with its verdict, open rims and seam family) | Required (`import`, `purge_unused`, `health`, `close`, `focus` Standard) |
| `hew.query` | `scene` (tree + per-entity summaries), `entity`, `faces` (planes, areas, centroids, boundary loops), `raycast`, `measure`, `resolve` (§5.3), `context` (the open frame stack as `stack`, the app's in-context entries as `fallback`, whether a tool gesture is armed as `armed`; headless: `fallback: []`, `armed: false`), `changes_since` (what changed since a history generation, §4.5), `occurrences`, `part_frame`, `mesh`, `topology` (§7.4) | Required (`changes_since`, `occurrences`, `part_frame`, `mesh`, `topology` Standard) |
| `hew.sketch` | `draw_line`, `draw_rect`, `draw_circle`, `draw_arc`, `draw_polygon`, `offset` (a sketch region, or a face/plane region by unit key), `units` (list a plane's drawing as units), `move`, `rotate`, `scale`, `copy`, `delete` (edit units on a plane) | Required (`units`, `move`, `rotate`, `scale`, `copy`, `delete` Standard) |
| `hew.solid` | `extrude` (region → new Object; also a plane region by unit key), `push_pull` (face of a solid), `union`, `subtract`, `intersect`, `slice` (`subtract`, `slice`, and `push_pull` also on a definition member through a path, below), `follow_me`, `imprints` (list an object's drawn shapes), `move_imprint`, `rotate_imprint`, `scale_imprint`, `delete_imprint` (edit or remove one; kept for compatibility, implemented over plane units, with `hew.sketch.units`/`move`/... as the general form), `cap`, `delete_faces`, `mark_surface`, `thicken` (repair open objects, see below), `from_faces` (Make Solid on drawn faces) | Required (`follow_me`, `imprints`, `move_imprint`, `rotate_imprint`, `scale_imprint`, `delete_imprint`, the repair commands and `from_faces` Standard) |
| `hew.compute` | `boolean`, `slice`, `shells`, `measure`, `outline`, `offset`: pure computations on transient solids that never change the document (§7.6); `project` and `stretch` are declared and refuse `unimplemented` until the flat-drawing projection and section stretch land | Standard |
| `hew.entity` | `rename` (objects, groups, instances, component definitions, materials), `delete` (also materials and component definitions — a definition dies with every instance that places it), `move` (with copy, array, and grid forms), `rotate`, `scale` (each also on members of one definition through a path, below), `set_frame` (a part's frame override, §7.4) | Required (`set_frame` Standard) |
| `hew.context` | `enter`, `exit` | Required |
| `hew.group` | `create`, `explode`, `reparent` | Required (`reparent` Standard) |
| `hew.component` | `create`, `place`, `make_unique`, `explode`, `rebase` (new definition axes, nothing moves), `flip_instance` (mirror one placement) (§7.4) | Standard |
| `hew.material` | `create` (color or texture), `paint`, `set_default`, `set_opacity`, `set_pbr` (metallic and roughness, 0 to 1; maps are kept), `position_texture`, `replace` | Standard |
| `hew.tag` | `create`, `assign`, `set_visible`, `delete`, `rename` | Standard |
| `hew.geo` | `get`, `set` (the document's geo-reference / coordinate reference system; `set` is one undo step) | Standard |
| `hew.guide` | `line`, `point`, `angular`, `clear` | Standard |
| `hew.scenes` | `list`, `add`, `update`, `rename`, `describe`, `remove`, `reorder`, `apply` (§7.1) | Standard |
| `hew.library` | `list`, `describe`, `insert`, `save`, `remove`, `update_meta` — the Hew Library (§8.1) | Standard |
| `hew.attr` | `get`, `set`, `delete` (§8) | Required |
| `hew.history` | `undo`, `redo`, `status` (depth; top entry's label and origin; the saved depth; every undo/redo entry's label and origin) | Required |
| `hew.view` | `snapshot` (render the attached document to PNG, headless or live), `camera` (set the live viewport's camera), `zoom_extents` (frame all visible geometry), `units` (set the app's displayed length-unit format), `line_drawing` (hidden-line SVG or segments, headless or live, §7.2), `hide`, `show`, `show_all`, `hidden`, `isolate`, `clear_isolation`, `zoom_to` (view state, §7.3), `enter`, `exit`, `exit_all` (the person's editing context, §6.3) | Standard (`core` grants six: `snapshot`, `line_drawing`, `hide`, `show`, `show_all`, and `hidden` — none needs a window, and the last four are kernel-served over the document's user-hidden flags; the rest stay `app`-only — live-host-only effects. `hew.view.hidden` reports `isolated: null` on a host with no isolation) |
| `hew.selection` | `get`, `set`, `clear`: the app's selection (§7.3) | Standard |
| `hew.print` | `pdf` (a full print job — paper/scale/tiling/marks — as a PDF, headless or live, §7.2) | Standard |
| `hew.annotate` | dimensions, leader text | Reserved |
| `hew.event` | `subscribe`, `unsubscribe`: server-originated notifications (§4.5) | Standard |

Semantics notes, normative:

- **The document's clock.** Every `hew.query.*` result and
  `hew.history.status` carry two integers as top-level members.
  `generation` is the history generation: it moves on every recorded
  change, undo, and redo, and an aborted transaction or compound rewinds
  it with the stacks it restores, so it identifies a history state and is
  what `expect_generation` compares (§6.5). `epoch` moves whenever the
  generation does and also on every abort, and never runs backwards, not
  even across a document replacement: a subscriber keyed on it never
  sees time go back. Writes that record no undo entry (tag visibility,
  hidden flags, palette and tag registry additions, Scenes; §6.4) move
  neither. Neither is serialized; a replacement document starts its
  generation at 0 and its epoch above the old document's.
- Drawing commands take a **plane spec**: `{"ground": true}`, an explicit
  `{origin, normal[, x_axis]}` frame, `{"face": <locator>}` (draw on a
  solid's face), or `{"sketch": id}` (extend an existing sketch). Points
  are 3D world coordinates and must lie on the plane within kernel
  tolerance; off-plane input is refused typed, never projected silently
  (on a face: `point_not_on_face`). Any point parameter accepts a
  derived-point locator (§5.3) in place of coordinates. A `{"sketch": id}`
  naming a **detached** sketch (below) is that sketch's own context:
  `hew.sketch.units` lists exactly its units, and the unit verbs,
  `hew.sketch.offset` and `hew.solid.extrude` with it act on that sketch
  alone; a plane or face spec never includes a detached sketch.
- **The plane is the drafting context.** With a `{"face": ...}` plane
  spec the draw goes through the drafting engine: everything drawn on the
  face's plane — the face edges of every solid with a face on it, the
  drawing kept on those faces, and the free drawing over the void — is one
  arrangement, and the new geometry is added to it as one undo step, then
  each edge is stored where it can be: an edge that closes a region over a
  face becomes that solid's face structure (`"topology"`, an imprint); a
  loose edge over a face (a free end, a line touching no other edge) is
  kept in a sketch owned by that solid and rides with its faces
  (`"owned"`); an edge over no face is free drawing (`"free"`, in a free
  sketch on that plane). A solid's own edges are fixed: drawing splits
  them where lines meet them but never moves them. Consequences: a loose
  line on a face, an open path that touches no edge, and a shape
  overhanging the face's edge are all accepted — an overhanging shape
  splits at the face's edge into a face on the solid and a region over the
  void, and the void region extrudes as a new Object
  (`hew.solid.extrude` with `plane`, below) while the face part
  push/pulls, in either order. Drawing on the ground or into a sketch
  (`{"ground"}`, `{origin,normal}`, `{"sketch"}`) keeps its sketch-mode
  behavior and result.
- A face-target draw's result is `{object_id, units}`: `object_id` the
  solid the face belongs to, and `units` the **plane units** the draw
  touched — every drawing edge lying along what was drawn, then every
  region whose boundary carries it — each `{kind: "edge"|"region", key:
  "edg_<hex16>"|"rgn_<hex16>", store: "topology"|"owned"|"free",
  object_id?, sketch_id?}`. Keys are geometry-derived (a 64-bit hash of
  the geometry, in the plane's own coordinates), stable until the
  geometry they name changes (not the per-sketch compound ids of §5.2: a
  unit key is a prefix and exactly 16 hex digits), and are the way to address a unit in
  `hew.sketch.units`/`move`/`rotate`/`scale`/`copy`/`delete`/`offset` and
  `hew.solid.extrude`. Face tokens are minted from the faces the draw
  produced, when it produced any: for a closed shape `"face"` (the face
  carrying the drawn region — the new sub-face, or the part of the shape
  that lies on the solid) and `"parent"` (a face the shape borders: the
  reshaped parent, now carrying the shape as a hole, or the face across a
  cut); for an open path `"a"` and `"b"` (the faces it borders). A draw
  that produced no face (a loose line, a shape wholly over the void) mints
  no token, and a later `{"$face": ...}` reference to one refuses
  `face_token_unknown`.
- **Faces and solids from drawing.** A loop on one plane is a face,
  whichever sketches hold its lines, and flat faces that enclose a space
  with no gap become a solid, in the same undo step as the edit that
  closed them; a shape that closes against one solid's face grows that
  solid (it gets a new id and keeps the old one's name and tags), and a
  detached shape is a solid of its own. This applies to API drawing as to
  pointer drawing: `draw_line`, `draw_rect`, `draw_polygon`,
  `draw_circle`, `draw_arc`, and the unit edits `move`, `rotate`, `scale`
  and `copy` add two optional result fields — `faces_closed`, the
  sketches that now hold a face the edit closed on another plane (a face
  a solid consumed in the same step is not listed), and `solid:
  {object_id, grew}`, the first solid the edit made. A rectangle and four
  lines to an apex, each line on any plane through it, return `solid` on
  the closing `draw_line`. Planarity is exact: four points that miss a
  plane make no face. `hew.solid.from_faces {sketches}` is Make Solid for
  faces that are already drawn (a file drawn before this rule, or a shape
  whose closing edit was not a draw): the close pass first fills any loop
  of the named sketches' lines that is not yet a face, as a draw would;
  then every face the named sketches hold, and every face the pass made,
  seeds the search, which gathers the faces connected to them through
  shared edges, and the result is `{object_id, grew}` as above, one undo
  step labeled "Make Solid" (every face the pass made is kept, those of
  named shapes that stayed open included). Nothing made refuses with the
  document untouched: `loop_not_planar` when a gap's open edges close a loop that
  lies on no one plane (`detail.loops`, the loops' corners), else
  `faces_not_closed` with `detail.open_edges` (the count) and
  `detail.segments` (up to eight sample open edges, `[[x,y,z],[x,y,z]]`),
  else `faces_not_solid` when the faces close but build no solid
  (`detail.reason`: `overlapping_faces`, `inside_solid` — a shape drawn
  inside the solid it closes against makes nothing —, `self_touching`,
  `invalid`; with `overlapping_faces`, `detail.segments` gives up to
  eight edges two faces lie over each other along, `[[x,y,z],[x,y,z]]`).
  When faces lie over each other while open edges or a non-planar loop
  remain elsewhere, `faces_not_closed` and `loop_not_planar` name the
  overlap too: `detail.reason: "overlapping_faces"` and
  `detail.overlapping_segments` (up to eight edges), beside their own
  fields. An id that names no live sketch refuses `unknown_entity`. Two faces
  with the same loop on one plane (the same wall drawn twice, say once
  as a detached shape) are one face, not an overlap: the solid takes the
  ordinary one, else the earlier drawn, and the other stays a sketch.
- **Detached shapes.** `draw_line`, `draw_rect`, `draw_circle`,
  `draw_arc` and `draw_polygon` take an optional `detached` (default
  `false`). With `detached: true` on a `{"ground"}`, `{origin, normal}` or
  `{"face"}` plane spec, the shape is drawn as one new **detached**
  sketch: drawing that takes part in no arrangement with ordinary
  drawing, so it neither splits nor joins the face or the drawing under it
  (a circle centered on a face's corner stays a whole circle and the face
  stays whole) and nothing drawn later joins it. The one exception: an
  Object edge in its plane whose two ends are corners of the detached
  shape closes it. Extruding one of its regions always makes a new
  Object. Snapping still sees it. The result is the sketch shape
  `{sketch, region_ids, region_id?, curve_id?}` whatever the plane spec
  (no face tokens: no face changed), so a following `hew.solid.extrude`
  chains through `region_id` as on the ground. With a `{"sketch": id}`
  plane spec `detached` is refused as invalid params ("detached is decided
  by the sketch; omit it for a sketch target"): drawing into a sketch
  continues it, and the sketch's own flag decides. Free stores of a
  detached sketch carry `detached: true` in `hew.sketch.units` and in the
  `units` a unit edit answers, and `hew.query.scene` marks such a sketch
  `"detached": true`. A detached shape cannot be joined to a face later;
  delete it and draw again without `detached`.
- **Unit verbs.** `hew.sketch.units {plane}` (read-only) lists a plane's
  drawing: `units` — islands `{key: "isl_<hex16>", edges: [{key, a, b,
  store, object_id?, sketch_id?, curve?}]}` (`curve` is `{key:
  "crv_<hex16>", center, radius}` for a facet of an analytic circle) — and
  `regions` — `{key: "rgn_<hex16>", outer, holes, object_id?, face?,
  extrudable}` (`face` is a face locator `{object, at}` for a region that
  IS a face of a solid, ready for `hew.solid.push_pull`; `extrudable` is
  true for a region over no face). A solid's own edges appear in no
  island: they bound regions but belong to no unit. The edit verbs each
  take a required `plane` (any plane spec above, or a bare sketch id) and
  `units`, a non-empty list of keys: `isl_` an island (every drawing edge
  in it), `edg_` one edge, `crv_` every facet of the circle with that key,
  `rgn_` the region's ring edges that are drawing edges. `hew.sketch.move
  {plane, units, offset}` and `copy {plane, units, offset}` translate (or
  copy) by a vector; `rotate {plane, units, axis_point, angle}` turns about
  the plane's normal through `axis_point` (radians, counter-clockwise seen
  from the normal side); `scale {plane, units, center, factor | factors}`
  scales uniformly or per world axis (a scale that leaves the plane
  refuses `point_off_plane`); `delete {plane, units}` erases. Each is one
  undo step, edits the arrangement exactly as drawing there would — moving
  a shape onto another shape arranges it, on the ground too — and answers
  `{units}`, the plane units the edit made (new keys after the edit; empty
  for `delete`). `hew.sketch.offset` additionally accepts `{plane, region:
  "rgn_<hex16>", distance}` for a face or plane region (answering
  `{units}`); its original `{region: <sketch region id>, distance}` form is
  unchanged. A key that names no unit on that plane (or is malformed)
  refuses `unknown_unit`. A solid's own edges cannot be moved or deleted:
  an `edg_` key naming one, or an `rgn_` key whose ring is all solid
  edges, refuses `draft_boundary_edge_touched` and changes nothing.
- `hew.solid.extrude` also accepts `{plane, region: "rgn_<hex16>",
  distance}`, a plane region by unit key: a region over no face extrudes as
  a new Object (result `{object_id}`, face tokens per §5.4); a region that
  is a face of a solid push/pulls that face exactly as `hew.solid.push_pull`
  does (result `{object_id}` or, for a through-cut, `{object_ids}`); a
  region over a face that cannot hold it (a sliver kept as drawing)
  refuses `region_not_pushable`; a region over no face whose outer ring
  touches one of its holes at a point (a circle inside a rectangle with
  its rim on a side) refuses `region_touches_hole` — the shape inside it
  extrudes on its own.
- `hew.sketch.draw_arc` takes a `close`: `"open"` (default, a bare arc),
  `"pie"` (closed wedge — two radii to the center), or `"segment"`
  (closed circular segment — the chord) — the API counterpart of the UI
  Arc tool's Alt-cycled completion modes. `"pie"`/`"segment"` commit a
  closed profile exactly like `draw_rect`/`draw_circle`: a region in
  plane/sketch mode, a region arranged on the face in face mode.
  Refused typed when combined with a full-turn sweep, which is already a
  closed loop (the circle special case).
  `"pie"`/`"segment"` additionally need at least 2 chords: a single chord
  can't form a non-degenerate closed loop (a one-chord `"segment"`'s
  closing edge would retrace the same chord — no region at all; a
  one-chord `"pie"` risks a collinear, zero-area wedge at some sweeps,
  sharpest at a half turn). `segments` below that floor is refused
  static (`-32602`); the proportional default is floored at 2 as well,
  so a small sweep never silently falls into the same degenerate case.
  In face mode the arc's own facets keep their circle and the closing
  radii or chord are plain lines; a pie whose center lies on or beyond the
  face's edge is kept (it splits like any overhanging shape).
- In headless hosts `hew.doc.new`/`open` establish the working document.
  A live host at 1.0 keeps document lifecycle user-driven — the user
  creates or opens documents and clients `attach`; whether a live host
  additionally honors `new`/`open` (opening a fresh window, say) is
  host-defined and advertised via capabilities.
- `hew.doc.open` takes a `.hew` or a SketchUp `.skp` (the 2021-and-later container or a stream release, 3 to 8 or 2013 to 2020;
  read by content, chosen by the `.skp` extension). Opening a `.skp` is not an edit: it
  leaves no undo step. A headless host keeps the opened file as the
  source of the document's next `.skp` save; opening a `.hew` or
  `hew.doc.new` drops it. A file the reader cannot decode refuses
  `load_failed`.
- `hew.doc.save` and `hew.doc.export` take an optional `compat` boolean with format `"skp"`: it writes revision 17 of the 2021-and-later container (without layer folders, hatch patterns, PBR materials, environments, snaps, procedures, page extras and the coordinate system) so older SketchUp versions can open it; by default the lowest revision (17 or 18) that holds the content is written. `compat` with any other target refuses `format_mismatch`. SketchUp 2021 to 2025 are supported: revision 17 is the one to hand them.
- `hew.doc.save` takes an optional `format`: `"hew"`, `"skp"` (the 2026
  container, what current SketchUp saves), or a stream release by its
  name: `"skp3"` to `"skp8"` and `"skp2013"` to `"skp2020"` (SketchUp 3 to
  8 and 2013 to 2020, each in its own release's format; a file opened from
  one of them is remembered as that release, and a file of a release
  Skpwright does not list as the nearest one it writes). Without it the path's extension decides:
  a `.skp` path keeps the working document's own `.skp` container when it
  came from a `.skp` (the 2026 one otherwise), anything else writes
  `.hew`; with no path, the working file's own format is used. An
  explicit `format` that contradicts the path's extension (a `.skp` path
  with `"hew"`, or a non-`.skp` path with `"skp"`, `"skp3"` to `"skp8"` or `"skp2013"` to `"skp2020"`) refuses
  `format_mismatch` and writes nothing, so one format's bytes never land
  in the other's file. A `.skp` save builds on the file the document came
  from (or was last saved as), so unchanged content stays as SketchUp
  wrote it, and the written file becomes the new source; a `.hew` save
  drops it. The result carries `format` (what was written) and, for a
  `.skp`, `simplified` (what the file could not hold exactly) and
  `writer_notes` (what a stream-release writer dropped), both arrays of strings.
  In the desktop app a live save runs the app's own Save (Save As with a
  `path`, a saved copy with `copy`), so a `.skp` path or format writes
  exactly what File ▸ Save would. A live host that would only hand the
  bytes back (no `path`, nothing for the app to write) refuses a `.skp`
  format with `host_capability_missing`; `hew.doc.export` with `"skp"` /
  `"skp3"` to `"skp2020"` hands back a SketchUp file instead.
- `hew.doc.revert {document?}` is File ▸ Revert to Saved: it re-reads the
  document's saved file in place, discarding every change since the last
  save. The undo and redo history is cleared and the document reads clean;
  a live window keeps its camera and panels. There is no confirmation
  step (the app's menu item asks a person; the API caller has already
  decided). It refuses `nothing_to_revert` when the document has no saved
  file or has no changes since the last save, and `load_failed` when the
  file can no longer be read (the document is then left as it was).
  "Changed" is judged differently on the two paths: headless revert asks
  the undo history's saved mark (the kernel's `at_saved_mark`), so state
  the history cannot see, such as scene edits, does not count; the live
  app asks its dirty flag, which does count them. The desktop app routes
  it to the window like `hew.doc.save`.
- `hew.doc.export` also writes `"skp"`, `"skp3"` to `"skp8"` and `"skp2013"` to `"skp2020"`, as `save` does
  but never changing the document's own file or source.
- A SketchUp file carries display units (SketchUp's Model Info > Units).
  `hew.doc.save` and `hew.doc.export` take an optional `units`, one of
  `hew.view.units`'s formats (`"m"|"cm"|"mm"|"arch"|"frac_in"|"dec_in"`),
  that the written `.skp` opens in — a save back over a `.skp` included.
  Without it the rules are the app's File menu's: a save back keeps the
  file's own units, and a new file or an export takes the host's display
  units — the app's preference on a live host, meters (a new app's
  default) on a headless one, which has no preference of its own. An
  unknown name, or `units` on an export to a format with none to set,
  refuses `-32602` before the host hears of it; a `.hew` save ignores it.
- `hew.doc.import` merges a foreign-format file (STL, glTF/GLB, COLLADA,
  `.skp`) into the attached document as new entities, through the same
  importer crates and shared healing pipeline the UI uses — it is not
  `open`, which makes the file the working document. A `.skp` import's
  report carries the reader's `warnings` (damaged records it skipped).
  Import options mirror the UI's
  import-time questions as explicit parameters; notably STL, which
  carries no units, refuses typed unless `units` is given — there is no
  one to prompt.
  A `.png`, `.jpg` or `.jpeg` path places the picture instead, exactly
  as the app's Place Image does: a component definition named after the
  file (without its extension) holding one face painted with the
  picture, stretched over the face once, and one top-level instance of
  it. `origin` is the picture's lower-left corner, `u` the direction its
  bottom edge runs, `v` a direction toward its top (not parallel to
  `u`), and `width` its width in meters; the height follows from the
  picture's proportions. Defaults: the ground at the world origin, `u`
  `[1,0,0]`, `v` `[0,1,0]`, 1 m wide. The bytes are kept as they are; a
  picture over 64 megapixels, or one whose size cannot be read, refuses
  `load_failed`. The report adds `picture: {width, height}`.
- `hew.component.import_definition` `{bytes_base64, format, name?,
  units?, images?}` turns a file's bytes into one **unplaced** component
  definition and answers `{definition, reused, report: {objects,
  members}}`. `format` is `hew`, `skp`, `glb`, `gltf`, `stl`, or `dae`
  (a picture format refuses `unsupported_for_definition`); `units` and
  `images` mean what they do for `hew.doc.import`. There is no `path`
  member: the call carries the bytes, so a plugin needs no file
  permission, and it requires `model.write`, not `documents.save`. A
  `.hew` file is read directly; the other formats go through the host's
  importers into a scratch document, so nothing lands in the world and
  the import report is not kept. The whole world content of the file
  becomes the definition's members in the definition's own frame
  (nested definitions come along). The definition is stamped with the
  SHA-256 of the format, the `units`, the bytes, and the `images` (each
  image's uri, format, and bytes; `hew.library` provenance, a fixed
  definition marker): a later import of the same bytes in the same
  format with the same units and images answers the same `definition`
  with `reused: true`, adds no undo entry, and keeps the first import's
  name. A different `units` or `images` is a different definition. A new definition is
  one undo entry. Place it with `hew.component.place`. The file plus all
  its images over 64 MiB refuse `import_too_large` (checked on the
  encoded length, before anything is decoded); a name is at most 128 bytes with no control
  characters (default `"Imported"`); an empty file refuses `empty_group`,
  and it refuses `explode_session_scope` while an editing session is
  open. The live app records the whole frame for replay, so the file
  bytes ride the recording. It cannot be rehearsed in a dry run.
- `hew.solid.extrude` consumes a region exactly as the push/pull tool
  does: the profile becomes the new solid's base face and leaves the
  sketch. `hew.solid.push_pull` on an existing solid's face carries the
  tool's full semantics — through-cuts, translate-and-build on oblique
  neighbors, whole-wall radial offset on stamped curved walls — and its
  full refusal inventory.
- `hew.solid.follow_me` mirrors the tool: the profile is a sketch
  region (addressed exactly as `extrude`'s is) or a solid's face (a
  face locator); the path is an **edge chain** — an ordered list of
  edge locators, a face locator standing for that face's boundary loop,
  or a sketch curve id — validated for connectivity and refused typed
  where the tool itself refuses (branching selections, bends tighter
  than the profile). Exact schema in the registry.
- `hew.solid.union`, `hew.solid.subtract` and `hew.solid.intersect` take
  `a` and `b`, or `operands` (two or more node ids, in order) for one
  combine over all of them as one undo step: a Union of all, the first
  minus the union of the rest, the intersection of all. `operands` with
  fewer than two refuses `boolean_needs_two`; `a` without `b`, or both
  spellings at once, is a malformed request.
- `hew.solid.mend {object, weld?, folds?, split?, fill?, require_solid?}`
  is Model Health's Mend: merge points closer than `weld` meters (omitted
  or `true` = the object's default distance, `false`/`null` = none), remove
  zero-width folds, separate pinched parts (`split`; islands become objects
  of their own), and fill small gaps (`fill`: `"holes"`, a list of
  `hew.doc.health` rim indices, or `false`), as one undo step. It does what
  it can: a gap it cannot close is left open and listed in `skipped_rims`
  with its refusal code as the reason, and the result reports `closed`.
  `require_solid` refuses `still_open` instead of committing a result that
  is not watertight. Pieces carry the object's name and tags. It refuses
  rather than leave the object worse (a solid opened at a narrow neck, a
  thin part pressed flat, a face turned over) and with `nothing_to_mend`
  when no step has work; a refusal leaves the document untouched.
- **Editable imprints** (kept for compatibility; implemented over the
  drafting engine's plane units — `hew.sketch.units`, `move`, `rotate`,
  `scale`, `delete` are the general form and take plane and unit keys):
  an *imprint* is a drawn unit on one of an object's drawing planes
  (`Document::object_drawing_planes`: planes where drawing splits a face,
  plus planes of the object's owned sketches). A *sub-face* is a region
  over the object whose outer ring is wholly drawing edges and is a
  closed loop nothing else meets (no other line joins it, it does not
  touch the solid's boundary); a shape that other drawing cuts is listed
  as its chords instead. A *chord* is
  a connected run of drawing edges, anchored at both ends on the solid's
  boundary or on other drawing (a run with a free end is neither). Moving
  or deleting an imprint acts on its whole island of drawing edges.
  `hew.solid.imprints {object}` lists, per drawing plane, every
  sub-face — `{kind: "sub_face", at, loop, holes, curve, curves, nested,
  normal, region, unit}` — and every chord — `{kind: "chord", at, path,
  curves, normal, unit}`. `curves` is the per-edge circle each loop or run
  edge is a facet of (`{center, radius}` or `null`; `curve` names only a
  whole drawn circle), `normal` is the plane's normal, and `region` /
  `unit` are the `rgn_…` / `isl_…` keys `hew.sketch.units` reports for
  that plane. `at` is the **locator** to pass back to the four commands
  below: a point strictly inside a sub-face (clear of its holes), or the
  midpoint of a chord's first segment. `hew.solid.move_imprint`/
  `rotate_imprint`/`scale_imprint`/`delete_imprint` take `{imprint:
  <locator>, …}`: `{object, at}` picks, on each of the object's drawing
  planes the point lies on, the chord whose line the point touches, else
  the sub-face containing it; `{object, ray}` and `{"$face": …}` name a
  face and pick the sub-face over it. A locator that finds no drawing (a
  point on a plain face) refuses `not_an_imprint`. `move_imprint` takes an
  `offset` translation; `rotate_imprint` an `angle` (radians) about the
  plane's normal through a pivot point; `scale_imprint` a uniform `factor`
  (finite and nonzero; a negative factor turns the shape 180 degrees about
  the anchor and scales it by its magnitude; a mirror about a world axis of
  an axis-aligned drawing plane is `hew.sketch.scale` with a negative
  per-axis factor) about an anchor point — every
  point parameter accepting a derived-point locator (§5.3). Each of the four is one drafting gesture on the unit's
  whole island (one undo entry), so a move landing on other drawing or
  past the face's edge is accepted and lands as the same shape drawn
  there would. A landing off the plane (a move with a normal component,
  or a scale about a point off the plane) refuses `not_in_plane`; a
  solid's own edges never move (`draft_boundary_edge_touched`). A change
  that leaves the geometry identical (turning a square a quarter turn
  about its center) makes no undo entry. `delete_imprint` removes the
  island's drawing edges: a sub-face dissolves into its parent face and a
  chord's faces merge back. Result `{object_id}` for all four. The old
  face-topology refusals (`loop_not_strictly_inside`, `nested_not_flat`,
  `not_a_chord`, `endpoint_not_on_boundary`, `path_not_simple`,
  `would_corrupt`, `not_an_inner_face`, `loop_self_intersects`,
  `point_not_on_face`) no longer arise here. A circle or arc, whole or
  cut apart by other drawing, is also listed in `curve_units` beside
  `imprints` (always present, empty when there are none): `{closed,
  curve: {center, radius}, runs: [{at, path}]}`, one entry per curve with
  one run per cut piece; a run that is a listed chord carries that chord's
  `at`, `closed` for a whole circle, else an arc.
- The transform commands are contract-shaped as: `move` takes a
  translation vector or a from→to point pair; `rotate` takes a pivot
  point, an axis direction, and an angle; `scale` takes an anchor point
  and per-axis factors (finite and nonzero; a negative factor mirrors) — every point-typed parameter accepting a
  derived-point locator (§5.3). `move` and `rotate` copy with `copy`;
  exact schemas live in the registry. `copy: {count: n}` (the UI's
  array form) makes `n` copies at the committed step and its multiples:
  `step, 2·step, …, n·step`, `step` being the translation (or the
  rotation about the pivot). `copy: {grid: [{count, step}, …]}` (`move`
  only) makes a grid of one to three axes in one undo entry: `count` is
  the number of slots along an axis, the original's included, and
  `step` the vector between slots, so `[{count: 4, step}]` makes three
  copies (where `copy.count: 3` also makes three) and `[{count: 2, step:
  a}, {count: 3, step: b}]` makes the five other slots of a 2 × 3 field.
  A grid takes its steps from `grid`, so `translation`, `from`, and
  `to` are params errors beside it, as are `count` and `grid` together,
  `grid` on `rotate`, an axis with `count` 0, and a grid with no axis
  above 1. The copies are every index tuple `(i0, i1, i2)` in
  lexicographic order except `(0, 0, 0)`, each translated by `Σ i_a ·
  step_a`; `ids` lists them placement by placement, each block in `ids`
  order. `copy.count` makes at most 1000 copies; a grid's counts
  multiply to at most 1000 slots, the original's included, so at most
  999 copies. Past either cap the command refuses
  `array_count_too_large` (`detail.max: 1000`).
- `hew.group.reparent {ids, parent}` moves live world nodes into `parent`
  (a live group) or out to the top level (`parent: null`) without
  touching geometry — a group is a pose-less container, so this is pure
  tree bookkeeping, the API counterpart of the Outliner's drag-and-drop.
  A node already under `parent` is silently skipped; refused typed
  `group_cycle` if `parent` is one of the moved nodes or lies inside one,
  and `explode_session_scope` while any group- or component-edit session
  is open, matching `hew.context.enter`'s own scoping.
- Convenience/primitive commands (`create_box`, …) are deliberately
  absent from 1.0. If they are added later they will be composites defined
  over these core commands, adding no new kernel semantics; whether they
  arrive as core sugar or as a first-party plugin is an open product
  decision this spec does not preempt.
- `hew.view.snapshot` exists so a client can *see* the model rather than
  only query it — for an agent, the difference between knowing what is
  true and seeing what it built. Every host renders it through the same
  deterministic software rasterizer (no GPU, no viewport, bytes in and
  bytes out; §7.2), so the app and hew-cli agree — the command takes an explicit camera, a named standard
  view, or a Scene's id (`camera`, `view`, and `scene` mutually
  exclusive; giving none of the three renders the document's saved
  working camera if it has one, else a fitted isometric view — §7.1's
  `scene` renders through that Scene's OWN resolved camera and hidden
  sets instead) and returns PNG bytes base64-encoded by default, or
  writes them to an explicit `path` instead (below). `include_ids: true` also
  returns a per-pixel id-buffer (`u16` little-endian, 0 = background) and
  the palette of public ids it indexes, so a caller can ask "what object
  is at pixel (x, y)" machine-readably. Two more passes, from the same
  rasterizer pass and the same camera, come back as their own PNGs:
  `include_depth: true` a grayscale depth image — NEAR is WHITE, the
  background is 0, every drawn surface maps linearly into `1..=max`
  (`max` is 255, or 65535 with `depth_bits: 16`) between the farthest and
  nearest drawn surface — together with `depth_near`/`depth_far`, the
  meters from the eye along the view axis (not radial distance) those two
  ends stand for, so absolute depth is `depth_far - (v - 1) / (max - 1) *
  (depth_far - depth_near)`; and `include_normals: true` a camera-space
  normals image, `(n * 0.5 + 0.5) * 255` per axis with x right, y up, z
  toward the viewer, background `(128, 128, 255)`. These are the
  conditioning images depth- and normal-aware image models take, so a
  render pipeline needs no external depth estimator. Under perspective,
  geometry is cut at a near plane 1 cm in front of the eye — a long
  ceiling sheet that reaches behind a close camera is clipped to it, not
  dropped — in every pass; a render that draws nothing returns an
  all-background depth image over a zero range.
- `hew.history.undo`/`redo` act on the shared document history — the
  topmost entry, whoever authored it. Because that is a blunt instrument
  in a `--live` session, `undo` accepts an optional `expected_label`,
  refusing typed when the top entry isn't the named one, and
  `hew.history.status` reports the depth and the top entry's label and
  origin — so an agent verifies the top entry is its own before popping,
  rather than blindly undoing the user's last edit. `status` also
  reports `saved_depth` (the undo depth at which the document is clean,
  counting only real changes — session open/close entries are flagged
  `bookkeeping: true` in `entries` and are undoable but never dirty the
  document nor count as changes since the last save; `saved_depth` is
  therefore an index into `entries.undo` with those filtered out —
  `null` once a new action discarded the redo branch that held it; the
  app's dirty-title-dot and unsaved-changes dialog are built on the
  identical kernel state, `Document::at_saved_mark`) and `entries`, the
  full `{undo: [...], redo: [...]}` listing behind the app's Changes
  tray: every entry, oldest-first on `undo` and replay-order on `redo`,
  each with a **derived** `label` and `origin`. That derived label is
  not the same thing as `top`'s: `top.label` is `null` for any entry
  without a recorded `CompoundMeta` (a UI-authored edit, or one
  committed outside `commit_transaction`) — but every `entries` element
  always carries a human-readable label, because the kernel derives one
  from the action itself (`Document::describe_action`) when no
  transaction supplied one. History commands are
  solitary (§6.4) and sit outside the one-envelope-one-undo accounting.
  Undo through the API is subject to the same replay contract as the
  UI's (DEVELOPMENT.md rule 9): it either restores exactly or fails
  typed — the deferred `UnbuildPushPull` case (ROADMAP.md in this directory) is today's
  one documented typed-failure gap — and it never corrupts.
- `hew.doc.save` marks the document clean (`Document::mark_saved`, the
  same state `hew.history.status.saved_depth` reports) exactly when the
  write it triggered is actually complete: a host that writes the file
  itself (`hew-cli`'s `CliHost`) has finished by the time the command
  returns, so the mark is set there. A host with no filesystem of its
  own hands the bytes back instead (below) — the write hasn't happened
  yet at that point, so the command does not mark the document clean;
  the live desktop app calls the equivalent (`Scene::mark_saved`) itself
  once its own write (through Tauri, or a confirmed browser download)
  actually lands.
- `hew.doc.export` returns the exported bytes base64 by default; a
  client may instead pass `path`, honored by hosts with filesystem
  access (`hew-cli`, the desktop shell) and refused typed elsewhere.
  `hew.view.snapshot` follows the same posture: PNG bytes base64 by
  default, or a `path` a filesystem-capable host writes to instead —
  the inline encoding can exceed a client's tool-result budget at any
  useful resolution, so a caller that only needs the file on disk
  should ask for `path`. When `path` is given the result carries `path`
  in place of `png_base64`; `include_ids`'s id-buffer follows the same
  split, written to `<path>.ids.bin` (with its path returned as
  `id_buffer_path`) rather than base64-encoded inline — `id_palette`
  itself stays inline either way, since it is small. The depth and
  normals passes split the same way: `depth_png_base64` /
  `normals_png_base64` inline, or `<path>.depth.png` / `<path>.normals.png`
  (returned as `depth_path` / `normals_path`) with `path`; the metric
  range (`depth_near`, `depth_far`, `depth_bits`) is always inline.
- `hew.material.create` makes what the palette's "Add color" and "Add
  texture" make. A `texture` is `{bytes_base64 | path, world_size}`: a PNG
  or JPEG (recognized by its bytes; anything else is a params error) and
  the `[width, height]` in meters one tile covers. `color` is required
  without a texture; with one it defaults to white, so the image shows as
  it is, and a given color tints it. A `path` is read by `hew-cli`, never
  by the app (§12.2). `hew.query.scene` and `hew.query.entity` report a
  material's `texture` as `{format, world_size}`, or `null` for a flat
  color.
- `hew.material.position_texture` is the Position Texture tool:
  `{face, pins: {origin, u, v}}` lays one texture tile on the face with
  its corners at `origin`, one tile along the width at `u`, and one along
  the height at `v`, which sets scale, rotation and skew together. The
  pins are in the face's object frame (a locator's `at` frame) and are
  projected onto the face's plane, as the tool's pins are. Pins that don't
  span a tile refuse `degenerate_pins`. `{face, reset: true}` returns the
  face to the default projection at the material's tile size. Each is one
  undo entry, or none when the face is already positioned that way. Positioning is per face and independent of the material, so
  it also works on a face with no texture yet. `hew.query.faces` reports
  each face's `texture_pins` in the same form, or `null` for the default.
- `hew.material.replace {from, to, object?}` is the Paint tool's
  Shift-click: every face and object base painted `from` becomes `to`
  across the document (instance materials too, §7.5), or only within
  `object` (Ctrl/Cmd+Shift-click).
  `null` on either side means unpainted, and both keys are required. It is
  one undo entry, or none when nothing matches.
- `hew.entity.rename`/`hew.entity.delete` reach a palette material
  (`mat_…`) or a component definition (`cmp_…`) the same way they reach
  any other entity kind — no separate namespace. A material's `name` is a
  required kernel `String`, unlike a node's optional display name, so
  renaming a material with `name: null` is a params error, not a kernel
  refusal. Deleting a material clears every face/object-base reference to
  it (live and tombstoned rows alike) and tombstones it — the handle stays
  valid for undo, but it drops out of `hew.query.scene`'s `materials` list
  and out of `save()`. Deleting a component definition deletes every
  instance that places it (world and nested) as the SAME one undo entry,
  and refuses `definition_nested_in_definition` when a live definition
  still places it as a member — explode or make-unique that parent first,
  or delete the parent instead.
- `hew.doc.purge_unused` deletes every palette material and component
  definition nothing references — materials over every object row (live
  or tombstoned, so a purge can never leave a reference dangling even
  through an undo that revives a deleted object), definitions
  transitively (one only reachable through another unused definition is
  purged too) — as ONE undo entry, and adds no entry at all when there is
  nothing to purge. Returns `{materials, definitions}`, the counts
  actually removed.
- **Model health.** An object is a solid (watertight) or open. Imports
  are where open objects come from (§2), and an open object is one of:
  a **surface** (`flat_sheet`, `bent_sheet` — a single sheet drawn on
  purpose, not a defect), or open and needing attention: `cappable` (one
  flat opening around the whole object, a box missing its lid),
  `holes` (only openings smaller than the object), `mixed`, or
  `non_manifold` (the open edge branches, or does not close; no repair
  trusts it). `hew.query.scene` and `hew.query.entity` report this as
  `health` beside `watertight` (an instance: the most serious verdict
  among its definition's members); `watertight` keeps its meaning, false
  for a surface. `hew.doc.health` lists every open object — world objects
  and component-definition members, a member once however often it is
  placed — with its verdict, its open rims (`role`, `planar`, `cappable`
  — a face can close it —, `closed`, `perimeter`, `extent`, `edges`), its
  seam family (objects an importer
  split from one shape, joined by shared rims), whether it is marked as a
  surface, and counts by kind; `objects` narrows it. It is read-only.
- The repairs each take an `object` (a world object or a definition
  member; a member's repair reaches every instance), keep its id, and are
  one undo entry; a batch is one `hew.doc.transact`. `hew.solid.cap`
  closes the object's flat openings with faces (`rims` indexes
  `hew.doc.health`'s rims; omitted, every `cappable` rim — or, when none
  is, all of them, so the refusal says why), nesting coplanar openings so
  a pipe's open end closes as a ring, and turns an island the caps close
  inside out right side out (`reversed`; a cavity inside another island
  stays a cavity). The new
  faces are face tokens `cap.1`, `cap.2`, … in a transaction. It refuses
  `rim_not_planar`, `rim_self_intersecting`, `cap_would_be_coplanar` (a
  flat surface: thicken it instead), `non_manifold_rim`, `caps_overlap`
  (the face would cover another opening on its plane), `unknown_rim` and
  `object_is_solid`. `hew.solid.thicken` pulls a flat surface into a
  solid `distance` thick along each face's front normal (negative: behind
  it; `centered`: half each way), keeping its faces, paint and texture
  placement on both sides; separate pieces of the surface become solids
  of their own (`object_ids`). Anything but a flat surface refuses
  `not_a_flat_sheet`; under a micron refuses `thickness_too_small`.
  `hew.solid.delete_faces` removes faces (locators on that object) from an
  open object; islands cut loose become objects of their own beside it
  (`object_ids`), and removing every face — or naming none — deletes it
  (`deleted`), a component member too unless it is its component's last
  object (`last_definition_member`). A solid
  refuses `object_is_solid`: removing a face from a solid would make it
  leaky, the stance of §2. `hew.solid.mark_surface` marks an open object
  as a surface on purpose (`accepted: false` clears it): stored in the
  object's `hew.health` attributes, no geometry, and `hew.doc.health`
  keeps listing it with `accepted: true`.
- **Transforming a part through a path.** `hew.entity.move`, `rotate`,
  and `scale` take each `ids` entry as a public id or an occurrence
  locator `{"path": [instance, …, member]}` (§5.2, §7.4). One call is one
  scope: every entry names a world node (or sketch), or every entry names
  a member of ONE definition reached by the SAME instance path; a mix, two
  definitions, or two paths to one definition is `-32602` ("ids must all
  be world nodes or members of one definition reached by the same path").
  A path whose leaf is a world node counts as that node's id; a path that
  leads nowhere refuses `unknown_occurrence`. The gesture stays in world
  terms — `translation`, `from`/`to`, `pivot`, `axis`, and `anchor` are
  world values, exactly as for a world node — and Hew applies it in the
  definition's own frame (`t_local = P⁻¹ · t · P`, `P` the path's
  composed pose), so the member moves where the user would have dragged
  it under that placement, and every other placement of the definition
  follows with its own pose (a mirrored placement shows the mirror image
  of the move). A member group's leaves bake once and a member instance's
  pose composes, as for a world selection. `copy` (a count or a grid)
  clones the members inside the same definition, so every placement
  shows the copies; the result's `ids` are the copies' ids. Each call is
  one undo entry. While the definition — or one the path passes through —
  is open for editing, its members are world nodes, and the path form
  refuses `explode_session_scope`: transform the world rows instead. A
  copy also refuses `explode_session_scope` while any editing session is
  open, even one on an unrelated definition, because closing a session
  folds every node made since it opened into the open container. A member that left
  the definition between resolution and the edit refuses
  `unknown_entity`.
- **Cutting a part through a path.** `hew.solid.subtract` takes `a` as
  `{"path": [instance, …, object]}`: with `b` a world object, the cutter
  is mapped into the definition's frame through the path's composed pose,
  the subtract runs inside the definition, and the cutter is consumed;
  every placement shows the cut, and the result is a member that keeps
  `a`'s name, tags, and attributes (`result` is its id). `b` may instead
  be a transient cutter, a `SolidSpec` `{prism}` or `{box}` in world
  coordinates (§7.6): the same cut, one undo entry, and no object is
  created for the cutter, so a machining is one call (undo brings back
  only the host). A transient cutter beside a world `a`, or under
  `union` or `intersect`, is `-32602`. With `b` a member
  of the same definition by the same path (`{"path": […]}`), `union`,
  `subtract`, and `intersect` all run between the two members inside the
  definition. A union or intersect of a path member with a world object
  refuses `-32602`: the result would move world material into a shared
  definition, which unlinking is for; and `b` must not be a path member
  when `a` is a world node. `hew.solid.slice` takes `object` as a path to
  a member; the `plane` stays in world coordinates and both pieces stay in
  the definition. `hew.solid.push_pull` takes a face locator whose
  `object` is a path to a member; `at` and `ray` stay in that object's own
  frame, as `hew.query.raycast` reports them, and the push reaches every
  placement as a definition member's always does. A host or object inside
  a member group refuses `grouped_operand` (for a boolean between two
  members, either one). The on-loan rule above applies to all of them,
  and the ones that make new members, a subtract with a world `b`, a
  boolean between two members, and a slice, also refuse
  `explode_session_scope` while any editing session is open, as a copy
  does.
- `hew.entity.move` with `copy` over a Sketch selection refuses typed at
  1.0, whatever the count: the UI's sketch copy is tool-layer replay
  through the sticky rules, and the kernel-side sketch duplicate op does
  not exist yet (ROADMAP.md in this directory). The refusal is named in `move`'s registry
  inventory rather than silently diverging from this table's claim.
- `hew.view.camera` and `hew.view.zoom_extents` aim a *live* viewport —
  the gap `hew.view.snapshot` alone cannot close. Rendering a picture is
  not the same as putting the model where a human, or an agent watching
  over someone's shoulder, is actually looking: a 24 cm model on a
  metre-scale default grid is an invisible speck until something moves
  the camera to it. `hew.view.camera` takes the identical `camera`/`view`
  vocabulary `hew.view.snapshot` accepts — one camera spec in the
  protocol, not two — except exactly one of them is required (there is
  no document-camera fallback to give neither for, unlike a snapshot's
  "no camera given" default). `hew.view.zoom_extents` takes no
  parameters and frames whatever is currently visible, mirroring the
  app's own View > Zoom Extents; like the live viewport's own
  implementation, an empty scene is a harmless no-op, not a refusal.
  Neither command edits the document — both are `mutates_document =
  false` — so neither is recorded, neither adds an undo entry, and
  neither triggers the resync a real mutation would. Both are
  meaningful only where a viewport exists: a headless host (`hew-cli`
  without `--live`, `NoHost`) refuses both `host_capability_missing`,
  and a headless client that wants to *see* the model still reaches for
  `hew.view.snapshot`, passing a camera per call instead of moving a
  persistent one.
- `hew.view.units` sets the app's displayed length-unit format —
  `app/src/settings/units.ts`'s `LengthFormat`
  (`"m"|"cm"|"mm"|"arch"|"frac_in"|"dec_in"`). This is deliberately an
  APP-LEVEL DISPLAY PREFERENCE, not document state: it governs how
  lengths are formatted in dialogs, the VCB, and dimension readouts, the
  same way a live camera pose governs how the model is framed — neither
  is modeled data, so neither is recorded, undoable, or serialized into
  `.hew`. It lives under `hew.view` rather than a new namespace for that
  same reason: `hew.view.*` is already "how the live app currently
  presents the model," and a display-unit format is exactly that kind of
  presentation state, not a sibling concern needing its own top-level
  area. A live host applies it through that module's own setter (never a
  direct storage write), so the desktop app's separate Settings window
  stays in sync via the existing cross-window broadcast; a headless host
  has no display preference to set and refuses `host_capability_missing`
  (a headless `.skp` save or export names its units per call instead).
  A live `.skp` export opens in this preference unless it names units.
- `hew.library.*` addresses an item by its stable `id` (the `hew.library`
  metadata's UUID) or by its library-relative path — either works
  anywhere an `item` parameter appears. `hew.library.insert` goes through
  the same `Document::insert_document` the UI's own library-insert flow
  uses: giving a provenance-bearing `item` (or `bytes_base64` with a
  `content_hash`) makes a second insert of the same item version reuse
  the in-document definition instead of copying it again, exactly as
  described in §8.1. A `material` item instead goes into the palette, as
  the Library's "Add to palette" does (`Document::insert_palette`): each
  of its materials is added, or reused when an identical one is already
  there, and the result lists them as `materials`; nothing is placed, and
  like `hew.material.create` it adds no undo entry. `hew.library.save`
  extracts a `selection` (wrapped as a component, UI parity), one palette
  `material` (a `material` item holding just that material, as the
  palette's Save to Library makes), or, with neither, the whole document
  (a `model` item); for a selection it stamps the saved selection's source
  nodes with the new item's provenance (`Document::stamp_library_source`)
  — a real document mutation, but not an undoable one, which is why it is
  `ReadOnly` class like `hew.scenes.*` rather than `ModelMutating`. A
  host lacking a library (no filesystem, or none configured) refuses
  every `hew.library.*` command `host_capability_missing`; `--live`'s own
  pre-resolve posture is §12.1. `hew.library.save`'s `source_doc`
  parameter is the `hew.library` metadata's `sourceDoc` key (§8.1):
  display-only "saved from" bookkeeping, an explicit value always wins,
  and absent one it defaults to `Host::working_document_path` — a
  headless `hew-cli dispatch ... --file model.hew` stamps `model.hew`
  automatically; a fresh, never-saved document (or a host with no notion
  of a current file) leaves the key off entirely. Two differences from
  the UI's own Save-to-Library flow, deliberately not replicated by this
  command: the UI always writes a `savedAt`/`sourceDoc` pair together
  (this command's `savedAt` is unconditional, `sourceDoc` conditional —
  the API surface a script drives has no equivalent of "the document the
  human currently has open" to fall back to when neither is known), and
  the UI renders and caches a content-hash-keyed PNG thumbnail after
  every save (`renderItemThumbnail`, `app/src/App.tsx`) — `hew.library.save`
  never does: there is no `hew.library.*` command for it today (a future
  one would go through `hew.view.snapshot`'s same software rasterizer),
  so an item saved through the API shows a generic/no-thumbnail tile in
  the browser until a UI session opens the library folder and renders one
  itself.

### 7.1 Scenes

A **Scene** is a named, saved view of the document: the camera, the
hidden-object set, the hidden-tag set, the section plane, and the
editor's display toggles, captured and restored together in one step —
SketchUp's Scenes/Pages idiom (`crates/kernel/src/scenes.rs`,
docs/design/scenes.md §3, §7). `hew.scenes.*` is the API surface over the
kernel's own Scene document: `list` (every Scene, in tab order), `add`
(capture a new one), `update` (re-capture an existing one's properties
from the document's current state), `rename`, `describe` (free-text
notes), `remove`, `reorder` (move within tab order), and `apply` (write a
Scene's captured state back into the document).

**View state, not modeled geometry — but still `mutates_document`.**
Every command here but `list` is `ModelMutating` (may ride a transaction)
and the registry marks `mutates_document = true`, so a host still
re-renders and re-syncs after one — but NONE of it is undoable: the
kernel calls behind these commands never record a kernel op
(`crates/kernel/src/scenes.rs`'s own module doc: "Everything here is view
state, outside undo history"), so a transaction's compound entry commits
empty and no undo entry lands, exactly like `hew.tag.create`/
`set_visible` (§6.4's registry-state carve-out). `hew.scenes.apply`
additionally is not a *dirtying* change (view activation, not an edit) —
`add`/`update`/`rename`/`describe`/`remove`/`reorder` are, in the sense
that a lost Scene is a lost hour of work even though it costs no undo
step.

**A Scene's `properties` decide what to (re-)capture — but the reported
`props` on `list`/`add`/`update` say what was ACTUALLY captured, not
merely what was requested.** `hew.scenes.add`'s `properties` (each of
`camera`, `hidden_nodes`, `hidden_tags`, `section`, `display`) defaults
to `true` for all five; `hew.scenes.update`'s defaults to whatever the
Scene already captures (re-capture, never silently widen). A camera,
given or reported, carries `parallel_height`: the height the view spans
at the target in meters, which is what a parallel view frames whatever
the eye distance (a SketchUp parallel scene keeps exactly this). Given
without it, a camera takes what its perspective view spans at the
target; the kernel never stores a height that is not finite and
positive. `camera` and
`display` are captured only when there is something to capture: an
explicit `camera` param, or — when none was given and the property is
requested — the document's own saved working camera
(`hew.view.snapshot`'s cameraless-path fallback); a brand-new document
with no working camera and no explicit `camera` param therefore reports
`props.camera: false` even though `properties.camera` defaulted to
`true`. The display property also carries the document's view style
(`view_style`: backdrop and axes, as a `.skp` scene's style sets them)
when the document has one; activating the Scene restores it. `hidden_nodes`, `hidden_tags`, and `section` always capture
something when requested — an empty set, or a captured-but-no-plane
`section: null` — so those three stay `true` whenever asked for. This
mirrors the kernel's own `Scene::props()`, which is literally "which
fields are `Some`," not a separately stored intent.

**Hidden sets are FULL sets, not deltas.** A captured `hidden_nodes`/
`hidden_tags` set names everything hidden AT CAPTURE TIME; anything
absent is visible, including geometry created after the Scene was saved.
Applying an old Scene never resurrects hidden state for objects it never
knew about.

**Public id shape.** Scenes are not entities (§5.1) — their public id is
`"scene_<hex>"`, not one of `public_id`'s entity prefixes.

**`hew.scenes.apply`** writes the Scene's captured kernel-side state —
the hidden-tag registry flags, the user-hidden node set, the section
plane — into the document (`Document::apply_scene`), and returns what
changed: `camera` (present only if captured), `section` (present,
possibly `null`, only if captured), and `hidden_object_ids`/
`hidden_instance_ids` (public ids, present as a pair only when
`hidden_nodes` or `hidden_tags` is captured — possibly empty arrays —
and OMITTED entirely, not empty-arrayed, when neither is, so a
camera-only Scene can never read as "show everything" and silently
un-hide what the user currently has hidden). After the kernel write, it
also notifies the host (`Host::scene_applied`) — a best-effort signal, not
a second veto point: the document mutation already happened whether or
not any host acts on it. A live host (`crates/wasm-api`'s `LiveHost`)
records a `ViewDirective::ActivateScene { sid }` the same way
`hew.view.camera`'s effect rides the live bridge (§7's `hew.view.camera`
note) — `app/src/api/liveBridge.ts` hands the sid to the app's own
Scene-activation logic (camera tween, panel/outliner sync) rather than
recomputing it from raw numbers; a headless host (`NoHost`, `hew-cli`'s
`CliHost`) has nothing further to do and no-ops successfully.

**`hew.view.snapshot`'s `scene` param** (§7) renders through a Scene's
own resolved state instead of the document's live one: its captured
camera when it has one (else the usual cameraless fallback — the
document's saved working camera, else a fitted isometric view, fitted to
what the SCENE leaves visible, not the whole document), and its resolved
hidden object/instance sets in place of the document's own live hidden
state. Mutually exclusive with `camera` and `view`. The Scene's section
plane, if any, is NOT rendered headlessly at 1.0 — clipping is a
viewport-renderer feature `crates/softrender`'s software rasterizer does
not implement yet; a snapshot through a Scene with an active section
plane still renders, unclipped.

Refusals: `unknown_scene` (a malformed or since-deleted Scene id, on any
command that takes one, including `hew.view.snapshot`'s `scene`),
`duplicate_scene_name`/`empty_scene_name` (`add`/`rename`'s naming
rules, mirroring the kernel's own `check_scene_name`).

### 7.2 Printing and line drawings

`hew.view.line_drawing` and `hew.print.pdf` are the headless counterparts
of **File ▸ Print…** and its Export dialog's SVG format
(docs/design/printing.md §7b, §9b): both render through
`crates/hlr`'s hidden-line-removal pass over tessellated geometry, the
same engine the desktop print pages and SVG export use, so a script gets
the identical visible-edge/silhouette/section-outline/hidden-line output
a person would see in the app. Under perspective, geometry is cut at a
near plane a thousandth of the eye–target distance in front of the eye, so
a long wall or ceiling that reaches behind a close camera still hides what
lies beyond it; what the cut leaves running off the page is trimmed to the
drawing's extent, which is also all `bounds` ever covers.

**`hew.view.line_drawing`** takes a camera, a named standard view, or a
Scene's id (the same three mutually-exclusive forms `hew.view.snapshot`
accepts) and returns either an SVG string — a true-size drawing in
millimeters at a given paper/model `scale` (default 1, full size) — or
raw 2D segments in view-plane meters, each tagged with its kind (`hard`,
`silhouette`, `soft`, `section`, `hidden`) and the public id of the
entity it belongs to. `include_hidden`/`include_soft` opt into hidden and
non-silhouette curved-wall lines, both off by default. `selection` draws
only the named placements (§7.4), the camera fitted to them: with no
`camera` or `view` the drawing is a fitted isometric over the selection,
never the document's saved camera. `selection` and `scene` are mutually
exclusive here as on snapshots. A `cmp_` root ignores tags and instance
visibility and honors only its members' own hidden flags (what the
Components panel thumbnail shows).

Every host that owns a document serves both view commands from one
renderer crate (`crates/headless-render`): `hew-cli` headless, and the
app's live host, which renders synchronously on its own thread (the
2048 px size clamp is the only bound). `hew.print.pdf` stays in
`hew-cli`.

**`hew.view.snapshot`'s `selection` and `background`.** `selection`
takes the same entries as export and line drawings (node ids,
occurrence locators, `cmp_` ids) and renders only those, so a plugin can
ask for a picture of one part, one definition, or any set of
placements. A `cmp_` id is one picture of the definition's members at the
identity pose, however many times it is placed; `include_ids` then
reports the definition's own id for those pixels, an instance's id for a
placement inside an instance. With `selection` and neither `camera` nor
`view` the camera is a fitted isometric over the selection, never the
document's saved working camera (which frames the whole model); `view`
fits that standard view to the selection; `camera` is used as given.
`selection` and `scene` are mutually exclusive, and an empty `selection`
is a params error. `background` is `"transparent"`, `#rrggbb`, or
`#rrggbbaa` (alpha 255 when omitted); it fills pixels no surface
touches, and the id, depth, and normals passes are unaffected. It
applies to snapshots only: a line drawing is vector and has no
background. The render is the software rasterizer's shaded look
(no textures, edges, or section planes), not the viewport's. A model too
complex for the vector pass refuses typed (`too_complex`) rather than
hanging or silently falling back — headless callers get an honest error
where the UI falls back to a raster page instead.

**`hew.print.pdf`** composes a full print job — standard (one page, the
view as given) or scaled (parallel projection at an exact drawing ratio,
tiled across pages with overlap bands, crop/trim marks, a scale bar, and
a title block) — and writes it as a PDF, exactly like the dialog's own
**Save PDF…**. `style: "line_art"` (the default) embeds the `hlr` output
as vector paths, refusing `too_complex` on a model the vector pass can't
handle rather than quietly rasterizing (unlike the interactive dialog,
which falls back to a bitmap page and says so — a headless caller gets a
typed refusal to act on instead); `style: "shaded"` rasterizes through
the same headless software renderer `hew.view.snapshot` uses, at lower
fidelity (no textures or antialiasing) than the live-viewport "As shown"
style. Bytes return base64 inline by default, or write to `path` on a
filesystem-capable host — the same split `hew.view.snapshot` and
`hew.doc.export` use, for the same reason.

Both commands are `Host`-implemented and headless-renderable, so — like
`hew.view.snapshot` — the `core` profile grants them even though they
sit outside `hew.view.*`'s usual app-only rule: `hew.view.line_drawing`
is named explicitly in `Profile::Core`'s carve-out; `hew.print.pdf` needs
no carve-out at all, since `hew.print.*` was never `hew.view`-prefixed to
begin with. A headless host (`hew-cli` without `--live`) renders both
without a viewport; MCP exposes them as `hew_print_pdf` and
`hew_line_drawing` whenever the connection's profile grants the
underlying command (§13).

Refusals: `host_capability_missing` (a host with no render path — should
not occur for `core`/`app`, both of which always grant a render route),
`nothing_to_render` (an empty document, or a degenerate camera),
`too_complex` (the `hlr` vector pass, on both commands — see above),
`save_failed` (a `path` the host couldn't write), `unknown_scene`.

### 7.3 Selection and view state

The app's selection, what is hidden, the viewport's isolation, and its
framing are view state: none of it is modeled data, none of it is undoable,
and none of it moves the history. Ten commands reach it, all `ReadOnly`
and all under the `view` permission; `mutates_document` is false for each.
Three more, `hew.view.enter`, `exit`, and `exit_all`, act on the person's
editing context instead (§6.3): they are solitary, so they cannot share a
transaction with another command.

| Command | Served | Params | Result | Refusals |
|---|---|---|---|---|
| `hew.selection.get` | host | `{}` | `{ids, primary, generation, epoch}` | `host_capability_missing` |
| `hew.selection.set` | host | `{ids, mode?}`: up to 10 000 public ids; `mode` is `replace` (the default), `add`, `subtract`, or `toggle` | the merged selection, as `get` | `unknown_entity`, `selection_target_not_visible`, `selection_mixed_kinds`, `host_capability_missing` |
| `hew.selection.clear` | host | `{}` | `{ids: [], primary: null, generation, epoch}` | `host_capability_missing` |
| `hew.view.hide` | kernel | `{ids}`: objects, groups, component instances | `{hidden}`: every hidden object, group, and instance | `unknown_entity`, `not_a_node` |
| `hew.view.show` | kernel | `{ids}` | `{hidden}` | `unknown_entity`, `not_a_node` |
| `hew.view.show_all` | kernel | `{}` | `{hidden: []}` | none |
| `hew.view.hidden` | kernel | `{}` | `{hidden, tags_hidden, isolated}` | none |
| `hew.view.isolate` | host | `{ids}`: 1 to 10 000 node ids or `{object, instance}` pairs | `{isolated}` | `unknown_entity`, `host_capability_missing` |
| `hew.view.clear_isolation` | host | `{}` | `{}` | `host_capability_missing` |
| `hew.view.zoom_to` | host | `{ids, padding?}`: `padding` from 0 to 2, default 0.1 | `{min, max}`: the framed box before padding | `unknown_entity`, `nothing_to_frame`, `host_capability_missing` |
| `hew.view.enter` | host, solitary | `{id}` or `{path}`, exactly one: a world object, group, or instance, or an occurrence path of instances whose last element, an instance or a member object, is the node to enter | `{}`; the outcome arrives as `context_changed` | `unknown_entity`, `unknown_occurrence`, `gesture_in_progress`, `host_capability_missing`; `-32602` for a sketch, guide, annotation, material, tag, or component target |
| `hew.view.exit` | host, solitary | `{}` | `{}` | `gesture_in_progress`, `nothing_to_exit`, `host_capability_missing` |
| `hew.view.exit_all` | host, solitary | `{}` | `{}` | `gesture_in_progress`, `host_capability_missing` |

**The selection.** `ids` are public ids in pick order; `primary` is the
first, or `null` for an empty selection. A selection holds objects,
groups, instances, and sketches in any mix, or exactly one guide, or
exactly one annotation; anything else refuses `selection_mixed_kinds`,
whether the picked ids mix or the merged result would (adding a guide to
nodes, or a second annotation to one).
An id that names no live entity of those six kinds refuses
`unknown_entity` with `detail.id`. A target that is hidden refuses
`selection_target_not_visible` with `detail.id`: a node is hidden when
it, a group above it, or a tag on it or on a group above it (or any
parent path of such a tag) is hidden; a sketch is hidden the way the
viewport hides it. Every id is checked before anything changes, so a
refused `set` leaves the selection as it was. The app's drafting
selections (units of a drawing on a plane) have no public id: while one
is selected, the selection reads as empty.

**The selection algebra.** `set` combines the picked ids with the
current selection, as the app's own Select tool does:

- `replace`: the picked ids, in pick order.
- `add`: the current selection, then the picked ids not yet in it, in
  pick order.
- `subtract`: the current selection without the picked ids.
- `toggle`: the current selection without the picked ids that were in
  it, then the picked ids that were not, in pick order.

Survivors keep their order, so the primary stays first unless it left.
An id named twice counts once. The result is the merged selection, which
the app then shows.

**Hidden state.** `hide` and `show` set the user-hidden flag of each node
named (every id is checked first; a sketch, guide, or anything else that
is not an object, group, or instance refuses `not_a_node`, and tags hide
through `hew.tag.set_visible`); `show_all` clears every one. The flags
are saved with the document, as the Outliner's eye saves them. `hidden`
lists the hidden nodes in creation order, the hidden tags by public id,
and `isolated`: the live viewport's isolation as a list of node ids and
`{object, instance}` pairs, or `null` when nothing is isolated or the
host has no viewport.

**Isolation and framing.** `isolate` shows only the named nodes, or a
definition member through one instance that places it (`{object,
instance}`; the object must be placed by that instance, directly or
through nested components), and frames them; it replaces any isolation
already on. Isolation is view-only and never saved. `zoom_to` frames the
union of the named entities' boxes (instances in world space; a guide
line, a material, or a tag adds nothing), widened on every side by
`padding` times the box's largest extent; a box with no extent (a lone
guide point) frames as a cube 1 m wide around it, padded the same way.
Nothing with a box refuses `nothing_to_frame`.

**View state and transactions.** All ten are legal inside a transaction,
read or write, and none adds an undo entry. An envelope that aborts takes
back what its view-state commands did: the hidden flags return with the
document, and the selection and isolation it set are never applied. A
`get` after a `set` in the same envelope reads the `set`'s result. A
plugin command that is aborted puts back the selection and isolation its
envelopes changed, and the hidden flags with the document; the camera a
`zoom_to` or `isolate` moved stays where it is.

**Where they work.** The selection, isolation, and framing need a host
with a viewport and a selection: the desktop app serves them, and
`hew-cli` headless refuses `host_capability_missing`. The web build and
Shop Mode have no bus clients yet and do not carry them out. `hide`, `show`, `show_all`, and `hidden` are served by the
kernel and are inside the `core` ceiling, as are `snapshot` and
`line_drawing`; the other view commands (`camera`, `zoom_extents`,
`units`, `isolate`, `clear_isolation`, `zoom_to`, `enter`, `exit`,
`exit_all`) are inside the `app` ceiling only. `hew.selection.*` is
inside `core` too and refuses there for want of a selection.

### 7.4 Parts: occurrences, frames, meshes, topology

These reads let a client list every part, measure it along its own axes,
and read its geometry. All four are `model.read`, kernel-served, and carry
the document's clock.

**`hew.query.occurrences`** `{roots?, kinds?, include_hidden?}` lists every
placement depth-first, members in their recorded order, nested
definitions expanded through each instance that places them. Each row:
`{path, id, kind, definition, definition_name, parent, world_pose,
mirrored, hidden, user_hidden, material, material_effective, tags, name,
depth}`. `path` is
the instances above the node (§5.2's occurrence paths); `world_pose` is
row-major 3×4, the node's own frame in world space: an instance's own pose
composed through its path, the path's pose for an object or group (their
geometry is already in their owner's frame); `mirrored` is that pose's
determinant sign. `hidden` is effective visibility: the node's own flag
and tags, its groups', and the same for every instance on the path;
`user_hidden` is the node's own flag. `name` is the node's own name,
never the definition's; `definition_name` is an instance's definition's
name (a list shows `name ?? definition_name`). `material` is an object's
default material or an instance's own material; `null` for groups.
`material_effective` is what the node's unpainted faces show: an
object's default material, else the material of the nearest instance on
`path` that has one (walking inward, the last); an instance's own, else
the same inherited material; `null` for a group (§7.5).

`roots` is a list of node ids (world nodes) and occurrence locators: rows
come in root order, each root's own row first, and a placement two roots
reach is listed once, under the first. Absent, the whole world is listed.
An empty list is a params error; a root that does not resolve refuses
`unknown_entity` or `unknown_occurrence`, and a `cmp_` id is not a root
(a definition has no placement). `kinds` filters the rows returned, never
what is traversed. `include_hidden` defaults to false: a hidden placement
is skipped and a hidden instance is not descended, which is the "as
displayed" view. While an editing session is open the walk reports the
model as the kernel holds it: a group session's members are world rows
and the group is a hidden row; an explode session's baked members are
world rows, and the entered instance and its siblings are hidden rows
that still list the definition under them at the pose each placement
would have. More than 200,000 rows refuses `too_many_occurrences`.

**`hew.query.part_frame`** `{id | path}` measures one placement:
`{origin, x, y, z, extents, source, mirrored}`, axes orthonormal and
right-handed in world space. An instance's frame is its definition's own
axes (`source: "definition"`), its pose's columns normalized, the
extents its definition's local box times the pose's per-axis scale, in
the definition's axis order; a mirrored pose negates `y`. An object or
group is measured along its largest face (`source: "face"`): `z` the
face's outward normal and `x` its longest outer edge, swapped so the
extents read `[length, width, thickness]` (`x` and `y` trade places, and
`z` turns over to stay right-handed, when the width is the longer by
more than float noise; a square part keeps its face frame). Ties break
on the part's own shape first, so the frame turns and moves with the
part; only faces or edges the part's symmetry makes interchangeable (the
top and bottom of a plain box) fall back to their world position, which
can reverse an axis's direction when such a part is turned. The frame
is the same after a save and reload. A `hew.part` / `frame` attribute
(`{origin, x, y}`, unit and perpendicular within 1e-6, else ignored)
replaces it (`source: "attribute"`); its values are coordinates in the
face frame this command reports for the part without the attribute, so
echoing that frame's own axes back (`origin: [0,0,0], x: [1,0,0], y:
[0,1,0]`) changes nothing. A placement with no face refuses
`no_part_frame`; one holding more than 200,000 placements refuses
`too_many_occurrences`.

**`hew.query.mesh`** `{id | path, space?}` returns one placement's
triangles: `positions_base64`, `normals_base64` (f32 little-endian xyz),
`indices_base64` (u32 little-endian, counter-clockwise from outside),
and `face_indices_base64` (u32 little-endian, one per triangle: the
face's index in
`hew.query.topology`'s `faces` plus its part's `face_index_base`), with
`vertex_count`, `triangle_count`, and `parts` (`{id, face_index_base,
index_base, index_count}` per leaf object, in walk order). `space` is
`world` (the default; normals by the inverse-transpose, winding flipped
under a mirrored pose) or `local`, the node's own frame: an object as
stored, a group's leaves as stored, an instance's leaves in definition
coordinates. Painted backs are not included. More than 64 MiB of buffers
refuses `mesh_too_large`.

**`hew.query.topology`** `{id | path, space?}` reads one object (a group
or instance refuses `not_an_object`): `vertices` in the object's stored
order, `faces` in the order `hew.query.faces` lists them (`{index, plane:
{point, normal}, outer, holes, material, back_material, surface, area,
attrs}`, loops as vertex indices, `surface` a cylinder's `{kind,
axis_point, axis, radius}` or `null`), and `edges` (`{index, a, b, faces,
curve, attrs}`, `faces` one or two face indices, `curve` a circle's
`{kind, center, axis, radius}` or `null`). `attrs` is the face's or
edge's attribute dictionary (§8), `{}` for none. Loops run counter-clockwise about the returned
`plane.normal` in either space; under a mirrored pose they are reversed
from the stored order to keep that true, as the mesh's winding is.
Indices are valid for the returned `generation` only. Under a pose that
is not a similarity, `surface` and `curve` are `null`.

**`hew.query.raycast`** `{origin, dir, all?, max_distance?, exclude?,
include_hidden?}` shoots a ray (meters, `dir` need not be unit) and
reports the part it hit through any depth of nesting. A hit is
`{object, kind, point, distance, normal, path, leaf, face, hidden}`:
`path` is the hit part's occurrence path without the part itself (the
form `hew.query.occurrences` uses; `[]` for a world object), `leaf` the
hit Object's id, and `face` a face locator `{object, at}` (§5.2) with
`at` in the leaf Object's own frame, so it feeds `hew.query.resolve`,
`hew.material.paint`, or `hew.solid.push_pull` as is. Painting by it
paints the shared definition's face, so every placement shows it. `object`
and `kind` keep their older meaning: the outermost instance of `path`
(`kind: "instance"`), else the world object (`kind: "object"`).
`distance` is world-space along the unit direction, and `normal` is the
face's outward world normal (a mirrored placement does not flip it). With
`all: true` the result is `{hits: [...]}`, every crossing nearest first,
front and back faces both (a ray through a board lists its entry and its
exit), ties in occurrence order, at most 1,000 (`raycast_hits_capped:
true` when more were found); an empty list is a success. Without it the
nearest crossing is returned; a tie within 1e-9 refuses
`ambiguous_locator` and nothing hit refuses `locator_missed`.
`max_distance` (finite, above zero) drops crossings farther than it.
`exclude` (at most 64 entries) is a list of world node ids and occurrence
locators `{path}`; each entry and everything under it is ignored, so
excluding one placement leaves the others of the same definition (a bare
definition-member id refuses `unknown_occurrence`). Hidden parts, whether
hidden directly, by a group, or by a tag, fall through as in rendering;
`include_hidden: true` lets the ray hit them and `hidden` then says so.
Sketches, guides, and annotations are never hit. More than 200,000
placements refuses `too_many_occurrences`.

**`hew.query.measure`** takes one of three forms. `{from, to}` is the
point-to-point form: `{distance, delta}`. `{id}` (a world object, group,
or instance) or `{path}` (an occurrence path) measures a part:
`{volume, area, bbox: {min, max}, extents, leaf_count, open_leaves}` in
meters, square meters, and cubic meters, summing the leaf objects under
the node (hidden ones included; a measure is not a view). Each leaf's
volume is scaled by its placement's `|determinant|`; `area` sums every
face polygon after the pose, holes subtracted, exact under any affine
pose including a mirrored one. A leaf that is not watertight adds no
volume and is counted in `open_leaves`; `volume` is `null` when no leaf
is watertight. `bbox` is the world-axis-aligned box of the vertices and
`extents` its sizes `[dx, dy, dz]`; the part-frame sizes are
`hew.query.part_frame`'s `extents`. A definition-member id without a path
refuses `unknown_occurrence`; any other mix of the three forms is a
params error.

**Setting axes.** Three `model.write` commands change how a part is
measured; each is one undo entry.

`hew.component.rebase` `{id, origin, x, y}` re-expresses a definition
(`cmp_`, or an `ins_` meaning its definition) in a new frame given in its
current local coordinates: `x` and `y` unit and perpendicular within
1e-9, `z = x × y`, so a left-handed frame cannot be stated. Every member
object, nested group, and drawing of the definition moves into the new
frame, every instance nested in it is re-posed to stay put, and every
instance placing it, world or nested in another definition, takes the
frame into its pose (`pose' = F · pose`), so world geometry is unchanged
and nothing moves on screen; a placement's `world_pose` changes, its
`bbox` does not. Result `{component, instances_updated}`, the latter the
number of placing instances. A frame that fails the check refuses
`invalid_frame`; while any editing session is open it refuses
`rebase_during_session`.

`hew.component.flip_instance` `{instance, axis: "x" | "y" | "z"}`
mirrors one world instance across the plane normal to its definition's
local `axis` through the local origin (`pose' = S · pose`, `S` the
reflection), the same pose-level mirror a negative scale gives. Result
`{instance, pose, mirrored}`; flipping twice restores the pose exactly.
A definition-owned (nested) instance refuses `unknown_entity`.

`hew.entity.set_frame` `{id, origin, x, y}` writes an object's or
group's `hew.part` / `frame` override (`{origin, x, y}`, stored as
given) after the same frame check, refusing `invalid_frame`; `{id,
clear: true}` removes it, and clearing an absent override records
nothing. The values are coordinates in the face frame
`hew.query.part_frame` reports for the part without the override, after
its `[length, width, thickness]` swap, exactly as `part_frame` reads
them. Result `{id, source}`: `"attribute"` after a set, `"face"` after a
clear. An instance or definition refuses `not_a_part` (a definition's
axes are `rebase`'s). This command is the one first-party writer of the
`hew.part` namespace; `hew.attr.set` on it stays subject to the
attribute namespace rule (§10.1).

**Tree and entity members.** `hew.query.scene`'s nodes carry `user_hidden`,
`hidden` (effective, within the node's own tree), `material` (an object's
default material, an instance's own material, else `null`), and `parent`
(the containing group);
instances add `pose` and `mirrored`. `hew.query.entity` on a component
lists `member_objects` (its member objects' ids, what `members` used to
mean), `members` (every member node as `{id, kind}`), and `bounds_local`
(its definition-local box, `null` when it has no leaves or more than one
walk lists); on an instance it adds `bbox`, `tags`,
`bounds_local`, `mirrored`, `material` (its own), and
`material_effective` (its own: an id names no path, so a nested
placement's inherited material is read from its occurrence rows).

### 7.5 Materials on faces, objects, and placements

A face shows its own material, else its object's default material, else,
for a member reached through instances, the material of the nearest
instance above it that has one; with none, the default gray. A painted
face or default material therefore keeps its color in every placement,
and an instance's material colors only what nobody painted (SketchUp's
rule).

- `hew.material.paint {face | id, material, scope?}`: `face` paints one
  face; an object id sets its default material; an instance id (world or
  nested) sets the placement's own material; a component id, or an
  instance id with `scope: "definition"`, sets the default material of
  every object the definition owns (its members and its member groups'
  objects, not nested components' geometry) as one undo entry. `scope`
  is `"instance"` (the default for an instance id) or `"definition"`; a
  component id takes only `"definition"` and an object id none, else
  `-32602`. `material: null` clears. Refuses `unknown_instance` /
  `unknown_component` for a stale target, `unknown_material`.
- `hew.material.replace` without `object` also switches every instance
  whose own material is `from` (a material only: `from: null` never
  matches an instance, whose `null` means "no material of its own").
- `hew.query.faces {object | path}`: with `path` (an occurrence path
  ending at an object, §5.2) each face also reports `material_effective`
  through that path. Every face reports `attrs`, its attribute dictionary
  (§8), `{}` for none. Refuses `unknown_occurrence` for a path that does
  not lead to a placement.
- Deleting a material clears it from instances too, and a material only
  an instance uses is in use: `hew.doc.purge_unused` keeps it, and
  palette usage counts each instance once.
- `hew.component.explode` gives every created object without a default
  material, and every created instance without a material of its own,
  the exploded instance's material, so nothing changes color.
  The SketchUp writers do not yet write instance materials.

**`selection` on export, snapshots, and line drawings.** `hew.doc.export`,
`hew.view.snapshot`, and `hew.view.line_drawing` take `selection`: node ids, occurrence locators,
and `cmp_` ids, which draw a definition's leaves at the identity pose. A
group gathers its leaves, an instance its definition's leaves through its
world pose. A leaf placement two entries reach (a group and one of its
members, an id named twice) is exported or drawn once. A selection over
the walk's 200,000-row cap refuses `too_many_occurrences`. Export keeps
its solids-only rule. The SketchUp formats write
the whole document, so `selection` with `skp` or `skp2017` is a params
error. An empty `selection` is a params error.

### 7.6 Compute

`hew.compute.*` are pure functions over solids: read-only,
`model.read`, served on every host (headless included), never creating an
entity and never returning an id. They are what a plugin calls at
pointer-move rate for a preview, a fit check, an outline, or a volume;
`dry_run` (§6.6) is the whole-envelope rehearsal before a commit.

**`SolidSpec`**, a solid operand, resolves to a transient solid in world
coordinates:

- `{ id | path, pose? }`: exactly one object (a group or instance refuses
  `not_an_object`: name its objects one at a time), at its world pose.
  `pose` (12 numbers, row-major 3×4) REPLACES the world pose. A pose that
  mirrors, or a mirrored placement measured without a `pose`, refuses
  `mirrored_pose` (solids never reflect; pass the un-mirrored frame and
  flip the answer yourself).
- `{ prism: { plane: { origin, normal }, loops: [[[x,y,z], …], …],
  distance } }`: built the way `hew.solid.extrude` builds a solid from
  drawn loops; the first loop is the outer boundary, the rest are holes,
  which must lie strictly inside it and apart from each other. Every
  loop is simple: one that crosses or touches itself (a bowtie, a loop
  through one point twice) or another loop is `-32602`, never a part of
  it kept. So are fewer than 3 points, a point off the plane, two
  consecutive points that coincide, a non-finite number, a zero
  `distance`, and a prism so large its area or volume is not a finite
  number. The message says which. The same loop rules hold for
  `hew.compute.offset`.
- `{ box: { min, max, pose? } }`: the prism of its bottom rectangle,
  `distance = max.z - min.z`, then `pose`; `min < max` on every axis.

Exactly one form per operand; an unknown member is `-32602`.

| Command | Params | Result |
|---|---|---|
| `hew.compute.boolean` | `{ op: "union" \| "subtract" \| "intersect", a, b, want? }` | `{ solid, mesh?, topology?, measure?, shells? }`; `solid: false` and no other block when nothing is left (an intersect of disjoint solids). The geometry is what the committed boolean would leave. `want` lists `mesh`, `topology`, `measure`, `shells` (a count); default `["measure"]`. Refusals are the boolean's by name (`degenerate_contact`, `nothing_removed`, `operand_not_solid`). |
| `hew.compute.slice` | `{ solid, plane: { origin, normal }, want? }` | `{ positive, negative }`, each with the wanted blocks; `positive` is the piece on the side the normal points to. Refuses `plane_misses_solid`, `degenerate`, `not_solid`. |
| `hew.compute.shells` | `{ solid, want_mesh? }` | `{ count, shells: [{ measure, mesh? }] }`: the solid's connected pieces. |
| `hew.compute.measure` | `{ solid }` | `{ volume, area, bbox: { min, max }, centroid }`; `volume` and `centroid` are `null` for a solid that is not watertight. |
| `hew.compute.outline` | `{ id \| path, frame?, which }` | `{ loops: [{ outer, edges }] }` in the frame's xy. `which: "largest_face"`: the loops of the part's largest face (ties: the one facing the frame's +z). `which: "projection"`: the silhouette of every face seen along the frame's z. `frame`: `"part"` (the default, `hew.query.part_frame`'s frame) or `{ origin, x, y, z }`, orthonormal and right-handed, else `invalid_frame`. Edges are `{ kind: "line", a, b }`, `{ kind: "arc", center, radius, start, end, ccw }`, or `{ kind: "circle", center, radius }`; a circle whose axis is the frame's z stays analytic, any other edge is a line. Outer loops run counter-clockwise, holes clockwise. |
| `hew.compute.offset` | `{ loops: [[[x, y], …], …], distance, join? }` | `{ loops }` in the outline shape; the first input loop is the outer boundary, in either winding. Positive grows the material. Refuses `offset_too_small`, `offset_collapsed`. `join: "miter"` is the default; `"round"` is reserved and `-32602` until the kernel has round joins. |
| `hew.compute.project` | `{ solid, plane, want? }` | declared, `implemented: false` until the flat-drawing projection lands |
| `hew.compute.stretch` | `{ paths, frame, axis, cutters, mode, distance \| dimension }` | declared, `implemented: false` until section stretch lands |

`mesh` and `topology` blocks have the shapes of `hew.query.mesh` and
`hew.query.topology` (§7.4) without `generation` and `epoch` (a
computation is not tied to a document state), with `parts[].id` and
`object` `null`. A `mesh` past 64 MiB of buffers refuses
`mesh_too_large`. `measure` is `{ volume, area, bbox }`.

**Caps.** The inputs of one call may hold at most 2,000,000 triangles,
counted from face loop sizes before any work (for a prism, six per loop
point, before it is built); past that the call refuses
`compute_too_large` with `detail: { triangles, limit }`. The loops of
one prism or offset hold at most 20,000 points (`-32602` past that). One computation
runs at a time per connection: a second call while one runs refuses
`compute_busy` (`detail: { limit: 1 }`). A connection's frames are served
one at a time today, so a caller that waits for each answer never sees
it; a host that serves computations off its dispatch thread holds the
connection's gate across each one. Send the newest preview; do not queue
older ones behind the pointer.

**Error codes**, the names this section and §6.6 add:

| Name | Code | `detail` |
|---|---|---|
| `dry_run_host_command` | `-32000` | `{ method }` |
| `compute_busy` | `-32000` | `{ limit: 1 }` |
| `compute_too_large` | `-32000` | `{ triangles, limit }` |
| `mirrored_pose` | `-32000` | `{}` |
| `invalid_frame` | `-32000` | `{}` |
| `dry_run` with `coalesce` | `-32602` | none |

## 8. Attribute dictionaries

Every Object, Group, Component definition, Instance, Sketch, Guide,
Material, Tag, and the document itself may carry **attribute
dictionaries**: named, namespaced bags of client data the kernel stores,
round-trips, and never interprets.

- A dictionary is addressed `(entity, namespace)`; a namespace is a
  reverse-DNS-style string owned by whoever coined it
  (`"com.example.shelving"`). The `hew` prefix (`"hew"`, `"hew.*"`) is
  reserved for first-party use: `hew.attr.set` and `hew.attr.delete`
  refuse it `reserved_attr_namespace`, matched exactly and
  case-sensitively, so a name that merely begins with those letters
  (`"hewlett.example"`) is a legitimate namespace and goes through.
  A restricted connection (§10) writes only under its own namespace: a
  plugin under its id and names below it (`com.example.cutlist`,
  `com.example.cutlist.parts`), plus any namespace its grant names with
  `attributes.write:<ns>`; a connected app only under the namespaces it
  declared when it paired. Anything else refuses
  `attr_namespace_not_granted`, with `detail: {ns, plugin, app}`. The
  `hew` reservation is checked first.
  Enforcement sits at the API boundary rather than in the kernel — the
  reservation exists so first-party code can claim those namespaces, so
  the kernel and the UI keep writing them freely. Reads are
  unrestricted; `hew.attr.get` returns first-party dictionaries like any
  other.
- Values are arbitrary JSON (finite numbers only), read and written at
  key granularity: `hew.attr.set {target, ns, key, value}`,
  `hew.attr.get {target, ns?}`, `hew.attr.delete {target, ns, key?}`.
- `target` is an entity's public id or `"document"`, or a solid's face or
  edge by locator: `{"face": <face locator>}` or `{"edge": <edge
  locator>}` (§5.2: `{object, at}`, and for faces `{object, ray}` or
  `{"$face": "label#key"}`). The locator resolves when the command runs,
  and a face or edge that is not found refuses as the locator does
  (`locator_missed`, `ambiguous_locator`). A sketch edge has no edge
  dictionary (store sketch data on the sketch) and refuses
  `not_a_solid_edge`. A target object naming both forms, or neither, is
  a params error. The namespace rules above apply unchanged. Face and
  edge dictionaries are reported by `hew.query.faces` and
  `hew.query.topology` as `attrs` on each face and edge, and change sets
  name them under the owning object with `scope` `"face"` or `"edge"`
  (§4.5).
- **How face and edge dictionaries travel.** A dictionary belongs to the
  element and moves with it through every operation, by one rule per
  kind of change:
  - An element that keeps its identity keeps its dictionary: a face moved
    by push/pull, an edge an operation reuses, everything a transform,
    copy, explode, make unique, or library insert carries.
  - A face built from an existing one takes its source's dictionary: both
    halves of a split, an imprinted shape's new face, every piece a
    re-trace makes, a boolean's or slice's result face from its operand
    face, both faces of a thickened sheet, and each face of a solid split
    into separate pieces. An edge split in two gives both pieces its
    dictionary.
  - An element built from nothing has none: push/pull and boss walls,
    extrusion and Follow Me faces, caps, and the edges of a cut or an
    imprinted loop.
  - A merge keeps the surviving element's dictionary and drops the other
    one, whatever their sizes (the survivor is the one that keeps its
    place; which one that is depends on the operation). Undo brings the
    dropped one back. A seam between coplanar faces whose dictionaries
    differ is part of the solid, as a paint boundary is: seam cleanup
    after a boolean leaves it, and drawing on the face treats it as the
    solid's own edge, which an erase refuses to remove.
  - Edge dictionaries do not survive booleans, slices, thickening a
    sheet, or splitting a solid into pieces: those rebuild every edge from
    positions, and the rebuilt edges start without one. Face dictionaries
    do survive them.
- Attribute writes are ordinary mutations: transactional, undoable, and
  serialized in `.hew`. Copy and instancing semantics follow the entity
  (a deep-copied group copies its dictionaries; instances carry their
  own, definitions theirs).
- Materials are not attributes. A face's, an object's, and an instance's
  material are document fields with their own commands and undo
  (§7.5); a plugin that wants per-part paint data beyond that (a finish
  code, a supplier's color name) keeps it in its own dictionary on the
  material or the part.
- Dictionaries survive round-trip through clients that do not understand
  them — that is their entire contract.

This is the extension point future plugins and overlays persist through —
a wall's BIM type, a plugin's per-object state — and it ships in 1.0
precisely because it is a native-file-format change (a manifest schema
addition), and format decisions are vastly cheaper before the 1.0 format
freezes than after.

### 8.1 First-party namespace: `hew.library`

The Library feature is the first first-party claimant of the reserved
prefix. Two dictionary sites, both ordinary attribute dictionaries with
the semantics above (round-tripped verbatim, readable by any client,
writable only by first-party code):

- **On a library item file's DOCUMENT dictionary** — the item's own
  metadata, written when the item is saved and edited by the library's
  manage flows: `id` (the item's stable library identity, a UUID string),
  `name`, `category` (`"component"` | `"material"` | `"model"`),
  `keywords` (list of strings), `collection`, `saved_at` /
  `sourceDoc`-style bookkeeping keys. The library browser derives
  everything it lists from this dictionary plus the manifest — there is
  deliberately no side database.
- **On entities a library insert creates** — provenance stamped by the
  kernel's insert (`Document::insert_document`) on created definitions
  and root nodes: `source_id` (the item's `id`), `content_hash` (a
  fingerprint of the item file's bytes), and on definitions `def_sid`
  (the definition's stable id in the item file). This is what makes
  re-inserting the same item version reuse the in-document definition
  instead of copying it again, and what "in this model" queries count.

A second first-party namespace, `hew.clipboard`, lives only inside
clipboard bytes and never in a document or a library file. The app's Copy
(`Document::extract_clipboard_item`) stamps every definition the copy
carries with `origin` (the copying window's document token) and `def_sid`
(the stable id of the definition it was copied from); Paste
(`Document::insert_document` with the same `origin`) links each stamped
definition back to the live original, so a component instance copied and
pasted within one document places the same definition — editing either
edits both, and Make Unique is the way to break the link. The insert
strips the namespace from every copy it makes, and a paste under another
origin (another window, or after the window opened another document)
copies as an ordinary item would.

Clients may read these dictionaries freely; external writes through
`hew.attr.set` remain refused like every `hew.*` namespace. The wire-level `.hew` encoding is specified in
HEW_FILE_FORMAT.md in the change that implements it (per that document's
same-commit rule and DEVELOPMENT.md rule 8); this section is the
semantic contract.

## 9. Discovery, schema, and versioning

`hew.meta.capabilities` returns the registry as data: for every command
granted to the connection's profile, its name, version, one-line summary,
JSON Schema for params and result, and refusal inventory. This is the
API's introspection surface — sufficient to generate a client SDK against
a running host, and the mechanism by which a client discovers what a
newer or older Hew supports.

The registry in `crates/api` is the single source of truth. From it are
generated, mechanically, all three artifacts this section promises — none
of them hand-written, and each traces back to the same `Registry::
protocol_1()` declarations:

- The MCP tool definitions `hew-cli` serves (`crates/hew-cli/src/
  mcp.rs`'s `generate_tools`) — computed live, at connection time, from
  whichever profile the connection was granted; there is no committed
  file for these, since a fresh registry read on every `tools/list` is
  the whole point (§13).
- The TypeScript client SDK used by the app and, later, by plugins
  (`app/src/api/hewApi.gen.ts`) — a typed `HewApiClient` over a
  caller-supplied transport, one method and one `Params`/`Result` type
  pair per command, grouped by namespace.
- The published API reference (`docs/API_REFERENCE.gen.md`) — one
  section per namespace, one entry per command: schemas, tier, class,
  served-by, and its refusal inventory with explanations drawn from the
  UI's own copy table wherever one exists.

The latter two are committed generated files (`crates/api/src/
codegen.rs`); `crates/api/tests/generate_artifacts.rs` is their
REGENERATE harness — regenerate both with `REGENERATE_API_ARTIFACTS=1
cargo test -p api --test generate_artifacts`, and a plain `cargo test -p
api` fails on drift between them and the registry (§14).

Evolution is **additive only**, protocol version 1 for as long as
possible:

- May appear in any release: new commands, new namespaces, new *optional*
  parameters (with defaults preserving prior behavior), new result
  fields, new refusal names, new locator forms, new profiles.
- May never change: the meaning of an existing command given existing
  parameters; the type or meaning of an existing field; a success into a
  refusal, or into a differently-shaped success, for the same input.
- A documented refusal MAY become a success in a later release — that is
  precisely how capability arrives (an `unimplemented` command gets
  implemented, a kernel gap closes, an eligibility gate widens). Clients
  must treat a refusal as terminal for the attempt, never as a permanent
  fact about the input.
- A change that cannot be additive ships as a successor command
  (`hew.solid.extrude_v2`) beside the original, and the original's
  deprecation is announced in the capabilities data long before any
  removal. Removal, if it ever happens, is a protocol-version event.

## 10. Grants

### 10.1 Grants and permissions

A grant is what a host lets a connection do, enforced at the dispatcher
(`-32001` outside it). Enforcement reaches **inside** transactions: every
command of a `hew.doc.transact` is checked against the connection's
grant statically, before any command executes, and a violation is
`-32001` carrying the offending index. `hew.doc.transact` itself grants
nothing — it is a container, not a capability. This is pinned at
protocol 1 because it cannot be retrofitted additively (§9).

A grant has a **ceiling** and a set of **permissions**. The ceiling is
the widest part of the registry the grant can reach:

- **`core`** — everything headless-safe: the full command surface except
  `hew.view.*` and the window commands `hew.doc.close`/`focus`, with six
  deliberate exceptions — `hew.view.snapshot` and `hew.view.line_drawing`
  have a headless render path (a software rasterizer, no GPU or live
  viewport required), and `hew.view.hide`, `show`, `show_all`, and
  `hidden` are served by the kernel over the document's hidden flags; all
  six are inside it. This is what `hew-cli` grants its
  own embedded kernel.
- **`app`** — `core` plus the commands that only make sense against a
  live application session: `hew.view.camera`, `hew.view.zoom_extents`,
  `hew.view.units`, the view-state commands of §7.3, `hew.doc.close`,
  and `hew.doc.focus` today. Granted by the desktop app to authenticated
  local connections.

First-party connections — the app's own UI, `hew-cli`, the MCP server,
and the desktop app's authenticated socket — hold the permission `all`
and reach everything inside their ceiling. For them nothing changed from
the two profiles protocol 1 started with: `hello` still reports
`"profile": "core"` or `"app"`, and `hello`, `hew.meta.capabilities`,
and every error answer byte-for-byte as before.

A plugin or a connected app holds a **restricted** grant: the
permissions its manifest or pairing declared, at the ceiling those
permissions derive (`app` if it holds `view`, any `documents.*`,
`files`, or `clipboard`; otherwise `core`). A restricted grant reaches a
command only if the command is inside the ceiling and its declared
permission is held. Every command declares one permission in the
registry (the `Permission` line of each entry in `API_REFERENCE.gen.md`):

| Permission | What it reaches |
|---|---|
| `always` | `hew.meta.hello`, `hew.meta.capabilities`, `hew.doc.attach` (to the connection's own document), `hew.doc.transact` (declaration only; every grant has it) |
| `model.read` | the read surface: `hew.query.*`, `hew.compute.*` (§7.6), `hew.attr.get`, `hew.meta.documents`, `hew.history.status`, scene and library listings, `hew.doc.health`, `hew.solid.imprints`, `hew.sketch.units`, `hew.view.snapshot`, `hew.view.line_drawing`, `hew.print.pdf` |
| `model.write` | every modeling command, `hew.scenes.add`/`update`/`rename`/`remove`/`reorder`/`apply`, `hew.history.undo`/`redo`, `hew.doc.purge_unused` |
| `attributes.write` | `hew.attr.set`, `hew.attr.delete` (under the namespace rule below) |
| `view` | `hew.view.camera`, `hew.view.zoom_extents`, `hew.view.units`, `hew.selection.*`, `hew.view.hide`/`show`/`show_all`/`hidden`/`isolate`/`clear_isolation`/`zoom_to` (§7.3), `hew.view.enter`/`exit`/`exit_all` (§6.3); the `selection_changed` and `tool_changed` events |
| `documents.save` | `hew.doc.save`, `hew.doc.export`, `hew.doc.import` |
| `documents.discard` | `hew.doc.close`, `hew.doc.revert` |
| `documents.other` | `hew.doc.new`, `hew.doc.open`, `hew.doc.focus`, the `document` parameter of `hew.doc.attach`, `close`, `revert`, and `focus`, and the envelope `document` member (§4.2) |
| `library` | `hew.library.insert`, `save`, `remove`, `update_meta` |
| `first_party` | declaration only: commands no restricted grant reaches |

A dry run (`hew.doc.transact` with `dry_run: true`, §6.6) changes
nothing, so it needs read permissions only: for each command it holds,
`model.write` and `attributes.write` are satisfied by `model.read`.
Every other permission is checked as for a commit, and the attribute
namespace rule still applies.

`files`, `clipboard`, and `network:<host>` are part of the vocabulary
for commands that later protocol revisions add; no protocol-1 command
declares them. `attributes.write:<ns>` grants writes under one more
namespace and also satisfies `attributes.write` for the command check.

A restricted connection's `hello` reports `"profile": "plugin"` (for a
connected app too) and adds a `grant` member echoing the grant:
`{ceiling, permissions, plugin, app, document}`, with `permissions` in
a fixed order. Its `hew.meta.capabilities` lists only the commands the
grant reaches, each entry with its `permission` and its `provider`
(`"kernel"` or `"host"` for a first-party command), and adds the same
`grant` member at the top level. The refusal names only a restricted
grant can meet (`document_not_granted`, `attr_namespace_not_granted`)
appear in a restricted connection's capabilities and in the published
reference, but not in a first-party connection's capabilities.

**Document binding.** A plugin's grant is bound to the document of the
window that owns it. Without `documents.other`, a restricted connection
may not use the envelope `document` member at all (`-32001`, data
`{refusal: "document_not_granted", document}`), and `hew.doc.attach`
with a `document` refuses `document_not_granted`; attaching with no
`document`, to the connection's own document, always works. The same
goes for the `document` parameter of `hew.doc.close`, `hew.doc.revert`,
and `hew.doc.focus`: naming one is `-32001` with data
`{refusal: "document_not_granted", document}` (`{refusal, index,
document}` inside a transaction), and leaving it out or `null` acts on
the connection's own document. A restricted grant never holds `all`:
a host refuses to open one that names it, and the dispatcher ignores it
if it is there. `hello` and
`hew.meta.documents` list only the bound document (every document with
`documents.other`), and leave out each entry's `path` unless the grant
holds `documents.save`.

**Path parameters.** A restricted connection may not set a parameter
that names a file on the host's disk: `path` on `hew.doc.open`, `save`,
`export`, `import`, `hew.view.snapshot`, `hew.view.line_drawing`, and
`hew.print.pdf`, and `texture.path` on `hew.material.create`. Setting
one, even to `null` or through a transaction `$ref`, is `-32001` with
data `{refusal: "path_not_granted", method, pointer}` for a plain
request, or `{refusal: "path_not_granted", index, pointer}` inside a
transaction, checked before anything runs. Byte-returning forms
(`return_bytes`, `bytes_base64`) stay available.

A grant is a *maximum* a host may narrow: a live host that keeps
document lifecycle user-driven simply withholds `hew.doc.new`/`open`
from its `app` grant. `hew.meta.capabilities` is the sole authority for
what a connection actually holds, and any registry command outside the
grant answers `-32001` with the message `method not in the granted
profile` — never `-32601`, which is reserved for methods the registry
has never heard of.

### 10.2 Plugin commands

A plugin's manifest commands (docs/agents/PLUGINS.md "Commands") enter
the same registry as Hew's own, named `<plugin-id>.<local>`
(`com.example.cutlist.generate`). A host registers them when the plugin
loads and removes them when it unloads; first-party commands are never
displaced, and a name that is already taken refuses the whole plugin.
Committed artifacts (the TypeScript SDK and `API_REFERENCE.gen.md`) never
list plugin commands.

`hew.meta.capabilities` lists a plugin command, to any grant that holds
its permission, with six members beyond the usual eight: `provider`
(`{"plugin": "<id>"}`), `permission`, `transactable` (always `false`),
`label` (its undo label, or `null`), `timeout_ms`, and `concurrency`
(`"parallel"` or `"serial"`). A read-only plugin command has class
`read_only`; a mutating one has class `solitary`.

**Forwarding.** The dispatcher never runs a plugin command. It applies the
checks every command gets, in the same order (unknown method, `hello`,
document binding, the grant, path parameters), then answers `-32002` if
the connection is not attached, then checks `params` against the
command's declared schema (`-32602`, message `params do not match
<method>'s declared schema: <pointer>: <why>`, naming the first mismatch
depth-first, with an object's members taken alphabetically by key). A call that passes is
handed back to the host as a forward; the host routes it to the plugin
and completes it with the plugin's reply, which the dispatcher shapes
into the response. A forward completes once.

| Plugin reply | Response |
|---|---|
| a result matching the declared result schema | the result (in the transaction shape for a sole-command transaction) |
| a result that does not match | `-32003` `plugin returned a result that does not match its declared schema`, data `{refusal: "plugin_bad_result", plugin, method, why, raw}` |
| a refusal `{name, explanation, detail}` | `-32000` `refused`, the canonical five fields with the refusal named `<plugin-id>.<name>` |
| no answer in time | `-32003` `the plugin did not answer in time`, data `{plugin, method, reason: "timeout"}` |
| the plugin stopped | `-32003` `the plugin stopped while handling this command`, data `{plugin, method, reason: "crashed"}` |
| the user cancelled | `-32000` `refused`, `plugin_command_cancelled` |
| the plugin is not running | `-32000` `refused`, `plugin_unavailable`, detail `{plugin}` |
| the plugin registers no handler | `-32601` `the plugin registers no handler for this command`, data `{plugin, method}` |

The app answers every forward `plugin_unavailable` until the plugin realm
lands.

**Read scope.** While a host serves a forwarded read-only command, it can
put the calling plugin's connection in a read scope: inside one, any
command that can change the document (a plugin's mutating command, a
first-party mutation, a transaction holding one) answers `-32001`,
message `this command is not available while serving a read-only
command`, data `{refusal: "read_only_forward"}`.

## 11. Transports

### 11.1 In-process

The UI and headless tools call `dispatch` directly; Rust and wasm callers
use the envelope types natively with no serialization. The UI is not
required to route through the public API at 1.0 — but the standing
direction is that the UI never performs a *model mutation* the bus cannot
express, so that a future migration of the UI onto the bus (the "first
overlay" end-state) is never structurally foreclosed.

While a compound is open (§6.1), the UI is refused too: every
document-changing call the app makes into the kernel fails typed
(`DocumentBusy`), and the input layer ignores pointer, keyboard, menu,
and palette input. After a quarter second the window shows a non-modal
notice naming the command, with Cancel, which aborts it.

### 11.2 Local socket (desktop)

The desktop app listens on a Unix domain socket (macOS, Linux) or named
pipe (Windows), carrying newline-delimited JSON-RPC frames. Discovery: at
startup the app writes
`<runtime-dir>/hew/instance-<pid>.json` — socket path, a per-launch
random 256-bit token, pid, app version — with owner-only permissions,
and removes it on exit (`<runtime-dir>` is `$XDG_RUNTIME_DIR` on Linux —
falling back to a per-uid `/tmp/hew-run-<uid>` where that variable is
unset, never a shared `/tmp/hew` any local user could claim first —
`~/Library/Application Support/Hew/run` on macOS, `%LOCALAPPDATA%\Hew\run`
on Windows). Both halves verify ownership before trusting the directory:
a discovery directory, discovery file, or advertised socket belonging to
another user is ignored, never read from and never deleted. The first
request on a connection must be `hello` with the token; everything else
is dropped, and the token is stripped from every frame before it reaches
the document — the renderer never sees the secret. The socket is never a TCP listener:
owner-only filesystem permissions plus the token mean nothing off the
machine, and no other user on it, can drive a document. On Windows the
same guarantee rests on a different mechanism, not a literal copy of
0600: the pipe is created with a security descriptor
(`D:P(A;;GA;;;OW)`) granting only its owner — the user Hew is running
as — access, since a named pipe is a kernel object with no filesystem
mode bits of its own, plus `FILE_FLAG_FIRST_PIPE_INSTANCE` on the very
first instance of the app's per-pid pipe name, so that another local
process cannot squat on that (predictable, pid-derived) name ahead of
launch — creation fails loudly instead of the app silently adding an
instance to the squatter's pipe object. The connecting client caps the
impersonation level it grants that pipe server to "identify" via
`CreateFileW`'s SQOS flags, and rejects a discovery file's `socket` field
outright unless it is the exact local shape `\\.\pipe\<name>` — a remote
UNC form (`\\<host>\pipe\...`) is never dialed. The discovery file's own
protection comes from `%LOCALAPPDATA%` already being a per-user directory
rather than an explicit ACL applied to the file itself; both halves
additionally confirm the runtime directory resolves (symlinks and
junctions followed) to somewhere actually under the current user's own
`%LOCALAPPDATA%` before trusting it, though — unlike unix's uid check —
neither queries the directory's discretionary ACL directly, so a
`%LOCALAPPDATA%` an administrator has made writable to other users is not
caught by this. As with unix's root, a local Administrator or SYSTEM can
still bypass a Windows DACL with sufficient privilege — neither
platform's transport tries to defeat that ceiling.

Window routing is the shell's, not the document's: each window's
webview owns one document and one dispatcher per client, and the shell
keeps the document registry (§4.2's ids), answers the host-wide commands
(`hew.meta.documents`, `attach` to a named document, `new`, `open`,
`close`, `focus`), and delivers every other frame to the window the
connection is attached to — or the one a frame's `"document"` member
names — by addressing that window directly, so a background window is
reached exactly as the front one is. A connection holds one session per
window it has touched, opened on first use by replaying its own `hello`
into that window; the `document` member is stripped before the frame
reaches the webview. A window that has not finished starting (a freshly
created one) is waited for, up to a bound, before the request is refused
`window_not_ready`.

A crashed instance leaves its discovery file behind, so discovery is
validate-then-use: a connector confirms the pid is alive, connects, and
completes `hello` before trusting an entry, and deletes stale files it
encounters; the app sweeps stale siblings at its own startup, following
an entry's socket path only when it names a file in the discovery
directory itself. Replies are correlated to requests: a connector
rejects a reply whose id is not the one it sent, and a host that gives
up waiting on a slow dispatch closes the connection rather than let a
late reply answer the next request — on Windows this final reply is
flushed through the pipe before it disconnects, since disconnecting a
named pipe (unlike closing a unix domain socket) discards any bytes the
client has not yet read. Connecting
to a dead socket is therefore a bug in the connector, not a hazard the
user meets.

A frame that arrives while a plugin command holds the document (§6.1)
waits in the window, in arrival order, and is dispatched once the
document is free. The host's 60-second reply clock restarts when the
frame is dispatched and each time a plugin command ahead of it finishes,
so a client is disconnected only if one command outlasts the clock, not
by the length of its wait; `hew-cli`'s own client clock is 120 seconds for the same
reason. When the host's clock does run out on a connection, it also
aborts any compound held for that connection's forward, so a closed
socket never leaves the document held.

### 11.3 stdio (MCP)

`hew-cli mcp` speaks MCP (itself JSON-RPC) on stdio to the AI client that
spawned it, and dispatches to its own embedded kernel by default — or,
with `--live`, forwards to a discovered running instance over §11.2
(§12). The adapter is nearly transparent because both sides are JSON-RPC
carrying the same schemas.

### 11.4 `postMessage` (Reserved)

The plugin transport: a Worker (web) or webview/child process (desktop)
exchanging the same envelopes over `postMessage`, isolation and
capability-granting per the sandboxed-plugin position of ARCHITECTURE.md
§4. Specified with the plugin system.

## 12. `hew-cli`

One binary, two modes, chosen per invocation:

- **Headless — the default.** `hew-cli` embeds `crates/api` and the
  kernel directly and never contacts, launches, or requires the desktop
  application. There is no UI process anywhere: documents are created or
  opened via `hew.doc.new`/`open` (or the flags below), mutated in
  memory, and land on disk via `save`/`export`. The embedded dispatcher
  grants the `core` profile.
- **Live (`--live`).** `hew-cli` discovers a running Hew instance
  through the discovery file (§11.2) and forwards every envelope to it:
  the client is now working on the document the user has open, in the
  user's undo history, in front of the user's eyes. With `--launch`, a
  desktop instance is started first and its socket awaited, for the
  agent-opens-the-app flow. Several live instances at once are an error
  listing the candidates, unless `--instance <id>` disambiguates. Live
  connections are granted the `app` profile by the application.
  `--document <ref>` (an id, path, label, or title; §4.2) attaches to
  that open document instead of the front window, in the background —
  with several documents open, the front window is rarely the right
  default for automation.

Headless is the default deliberately. Attaching to — and mutating — a
user's live document is the invasive act, and a CLI invocation or a
spawned MCP server that silently reached into an open GUI session would
be astonishing; opting into that is one explicit flag. Headless also has
the better failure mode — it works on a machine where no app is running
or even installed — and it is what scripts and CI want. The flagship
watch-it-build agent experience is not the default case; it is `--live`
in the MCP configuration, chosen once, on purpose.

Subcommands:

- `hew-cli mcp [--live [--launch]]` — serve MCP on stdio (§11.3, §13).
  Every tool also accepts a per-call `document` (the envelope member of
  §4.2).
- `hew-cli documents --live` — print the open documents, one JSON object
  per line (`hew.meta.documents`).
- `hew-cli events --live [--filter name,...] [--document <ref>]` —
  attach, subscribe to the named events (every name in §4.5 by default),
  print the subscribe reply to stderr, then print every frame that
  arrives to stdout as one compact JSON line until the app closes the
  connection (exit 0; any other failure exits 1). The other subcommands
  set aside any event that reaches them while they wait for a reply.
- `hew-cli run <script> [--out model.hew]` — headless script runner: the
  script is a JSON array (or JSONL stream) of envelopes, executed in
  order against the embedded kernel; refusals stop execution with a
  nonzero exit and the refusal on stderr as JSON. Because the kernel is
  deterministic, a script is a reproducible artifact — a logged agent
  session re-run as a regression test produces byte-identical output.
- `hew-cli dispatch <method> [params-json] (--file model.hew | --live)`
  — one envelope, result on stdout; the shell-scripting primitive.
  `--file` opens the document, applies the envelope, and saves in place
  (for a query, just reads); `--live` targets the running app. Neither
  given is an error with guidance, never a silent guess.

`hew-cli` links `crates/api` and the kernel directly; headless mode is
the same dispatcher with file I/O in the CLI host, per the purity rules.

### 12.1 Library commands in `--live`

A live host (`crates/wasm-api`'s `LiveHost`, behind the desktop app) has
no filesystem — the library folder lives on whichever machine `hew-cli`
itself runs on — so `hew-cli`'s live transport pre-resolves every
`hew.library.*` envelope on the CLIENT side before it reaches the wire,
for every live entry point (`dispatch --live`, `run --live`, `mcp
--live`) alike:

- `list`/`describe`/`remove`/`update_meta` never touch the document at
  all, so they are answered ENTIRELY by `hew-cli` itself — the exact
  `crates/api/src/commands/library.rs` handlers headless dispatch uses,
  run directly against `hew-cli`'s own `CliHost`, with no socket
  round-trip and no running desktop instance required. `LiveHost` itself
  refuses all four `host_capability_missing`, naming this pre-resolve as
  what a client should do instead — a raw (non-`hew-cli`) client reaches
  these the same way `hew-cli` does internally: resolve the library
  folder itself and never send these four over the wire at all.
- `insert {item}` rewrites `item` to `bytes_base64` (plus `content_hash`)
  before forwarding: `hew-cli` reads the item locally and hands the
  remote its bytes, since the remote's own host cannot read a path or an
  id it has no filesystem to resolve. `LiveHost::library_read` likewise
  refuses `host_capability_missing`, so `bytes_base64` is the only shape
  that works over `--live` regardless of client.
- `save` is forwarded with `return_bytes: true` forced (whatever the
  caller asked): `LiveHost::library_write` — unlike the other three
  `library_*` methods — does NOT refuse, because it needs no filesystem
  to do its part: it hashes the bytes (the same SHA-256
  `crates/library::sha256_hex` computes, so a live-stamped
  `content_hash` and the on-disk item's real hash always agree) and
  answers a placeholder `path` (nothing on the live side is a meaningful
  filesystem location). This is what lets `crates/api`'s handler complete
  `extract_item`/`stamp_library_source` against the LIVE document and
  hand real bytes back over an unmodified `--live` connection — a raw
  client gets a fully working save, just with that placeholder path,
  which is exactly why `hew-cli` takes the returned bytes and writes them
  to its OWN library folder, then reports the local `path` in place of
  the placeholder.

Either shape a mutating command can arrive in — bare, or as the sole
command of a one-command `hew.doc.transact` (§6.4's canonical MCP
invocation) — is handled identically.

### 12.2 Headless and live: what is the same, and what cannot be

The 81 commands the kernel serves run the same dispatcher over the same
kernel in both modes — a script's replies are the same, byte for byte
(exports included), except where §12.2's last list says otherwise. The 18
host-served commands are where the two hosts differ, and each is either
at parity or a stated, reasoned difference:

| Command | Headless | Live (desktop app) |
|---|---|---|
| `hew.meta.documents` | the one working document, `doc_1` | every open window (§4.2) |
| `hew.doc.new` / `open` | replaces the working document | a NEW window; never replaces one |
| `hew.doc.save` | writes the file, clears unsaved, binds the document to it | the same: the app saves through its own save path (title, recents, dirty mark), for a named `path` and for a document that already has a file |
| `hew.doc.revert` | reloads the saved file; clears history and the unsaved mark; refuses `nothing_to_revert` with no file or no changes | the same: the window re-reads its file through its own open path, keeping camera and panels |
| `hew.doc.save {copy}` | writes a copy; the document stays unbound and dirty | the same |
| `hew.doc.save`, no path, never saved | refuses `path_required` | hands the bytes back (the app cannot ask where) |
| `hew.doc.export` | writes the file | `hew-cli` writes it from the bytes the app returns |
| `hew.doc.import` | reads the file (and a COLLADA file's textures) | the same: `hew-cli` reads them and sends `bytes_base64`/`images`; the app merges additively, one undo entry, and records it for replay |
| `hew.component.import_definition` | decodes the bytes it is sent | the same; the definition is reused by content hash on both hosts, and the app records the whole frame for replay |
| `hew.material.create {texture: {path}}` | the host reads the image | the same: `hew-cli` reads it and sends `texture.bytes_base64` in place of `path` |
| `hew.view.snapshot`, `line_drawing` with `path` | writes the file | `hew-cli` sends the request without `path` and writes the bytes the app returns, once, answering the headless result (`path`, and the `.ids.bin`, `.depth.png`, `.normals.png` files for those passes); a raw socket client that sends `path` gets `host_capability_missing` |
| `hew.view.snapshot`, `line_drawing`, `hew.print.pdf` | software renderer | the app renders snapshot and line drawing; `hew.print.pdf`, and either of those on an older app that cannot render, `hew-cli` renders a copy of the document's saved bytes with the same renderer, from the camera the request names or the one the user is looking at |
| `hew.library.*` | the library folder | the library folder, on the client's side (§12.1); `save` records the document's file as `source_doc` as headless does |
| `hew.view.camera`, `zoom_extents`, `units` | refused (`core` profile) | take effect in the window |
| `hew.view.hide`, `show`, `show_all`, `hidden`, `isolate`, `clear_isolation`, `zoom_to` | refused (`core` profile) | take effect in the window (§7.3) |
| `hew.view.enter`, `exit`, `exit_all` | refused (`core` profile) | open and close the person's editing context (§6.3) |
| `hew.selection.*` | `host_capability_missing` | the window's selection (§7.3) |
| `hew.doc.close`, `focus` | refused (`core` profile) | §4.2 |

A headless host has no viewport, window, or app preferences, so the
`hew.view.*` effects and the window commands have nothing to act on there:
that difference is inherent, not a gap. A headless client chooses a camera
per `hew.view.snapshot` instead.

What differs and cannot be made to agree:

- **Float last digits after trigonometry.** `sin`/`cos` on native targets
  call the platform's libm; on `wasm32-unknown-unknown` the kernel carries
  its own, and the two can differ by one unit in the last place. A polygon
  or circle's vertices — and everything computed from them — can therefore
  differ at about 1e-16 between hosts. Making them identical means
  routing the kernel's trigonometry through one shared implementation on
  both targets, a change to the kernel's dependencies.
- **History origin.** Each edit's `origin` names the connection that made
  it (`hew-cli:run` headless, `live:N` in the app) — the identity differs
  because the connection does.
- **Segment order of a hidden-line drawing** (and so a PDF's drawing
  bytes) depends on how the document's edges were enumerated, which differs
  between a document built in memory and the same document reloaded from
  bytes. The set of lines is the same. This holds headless too, and a live
  client-side render always starts from saved bytes.

## 13. MCP mapping

The MCP server does not expose one tool per command — eighty near-flat
tools would bury an agent in choices while stripping the transaction
semantics that make multi-step work atomic. It serves a small set of
chunky tools, all generated from the registry:

- `hew_capabilities` — the introspection surface (§9), so an agent can
  learn the exact command schemas at run time.
- `hew_transact` — the workhorse: a full transaction envelope (§6),
  returning per-command results or the typed refusal.
- `hew_query` — the read surface: any `ReadOnly`-class command that does
  not mutate the document (§6.4 — `hew.query.*`, `hew.meta.*`,
  `hew.attr.get`, `hew.library.list`/`describe`/`remove`/`update_meta`,
  §8.1), plus `hew.view.snapshot`, `hew.view.line_drawing`, and
  `hew.history.status` by name. The view and selection commands in that
  class (`hew.view.hide`/`show`/`show_all`/`isolate`/`zoom_to`,
  `hew.selection.set`) change what is hidden, framed, or selected, never
  the geometry. `hew.library.save` is `ReadOnly` class but mutates the
  document (it stamps provenance), so `hew_query` refuses it.
- `hew_describe_scene` — `hew.query.scene` presented as a structured
  summary tuned for reasoning over (names, kinds, bounding boxes,
  watertightness, tree shape).
- `hew_snapshot` — `hew.view.snapshot`, so the agent can look at what it
  built, not merely query it. Present headless: `hew.view.snapshot` now
  has a headless render path (a software rasterizer), and `core` grants it
  specifically, so a headless MCP session lists this tool too.
- `hew_print_pdf` — `hew.print.pdf` (§7.2): the document printed to a PDF
  at an exact drawing scale, tiled and titled exactly as File ▸ Print…
  would, headless — bitmap pages through the software rasterizer or vector
  Line art through `crates/hlr`; `core` grants it the same way.
- `hew_line_drawing` — `hew.view.line_drawing` (§7.2): the hidden-line
  drawing as an SVG or as raw 2D segments for a camera, a standard view, or
  a Scene; `core` grants it the same way.

Plugin commands ride the same tools: `hew_query` runs a read-only plugin
command, `hew_transact` with a plugin command as its one command runs a
mutating one (§6.4), and `hew_capabilities` lists them with their
`provider`. A plugin command's result comes back labeled with the plugin
that produced it, so an agent can tell a plugin's answer from Hew's own:
an object result gains `"provider": "<plugin-id>"`, and any other result
becomes `{"provider": "<plugin-id>", "value": <result>}` (on `results[0]`
for `hew_transact`). With `--live`, the server learns the plugin commands
from the app's capabilities when it connects, subscribes to
`plugin_state` (§4.5), and reads the capabilities again after any tool
call during which a `plugin_state` event arrived, so the next call sees
the plugins as they are now.

The tool list is generated from the connection's granted profile: every
profile today grants `hew_snapshot`, `hew_print_pdf`, and
`hew_line_drawing`, since core's carve-outs are exactly these
headless-renderable commands; a future `hew.view.*` addition that stays
`app`-only would be the next tool a headless session lacks. An agent never
sees a tool it cannot call.

The intended agent loop is: describe → plan → `hew_transact` → read the
result or refusal → look → continue. Refusal explanations are the
self-correction signal; the transaction guarantee means a failed plan
leaves nothing half-built to clean up.

## 14. Conformance: tests precede implementation

The API is test-first in the classic sense: an executable conformance
suite, written from this document and the registry declarations, lands
**before** the dispatcher it tests, and implementation proceeds by
turning it green — never by adjusting it. DEVELOPMENT.md rule 5 applies
with full force; the suite is the contract's teeth.

The suite has four parts:

- **Golden transcripts.** Fixture files of literal request/response
  pairs — the handshake, `attach`, every error class of §4.4,
  transaction success and abort, `$ref` and derived-point resolution,
  context-balance rejection, profile enforcement — replayed against the
  dispatcher and compared *structurally*: parsed-JSON equality with
  exact `f64` bit equality, so a serializer's cosmetic choices can never
  break a golden. Kernel-served commands run against the bare
  dispatcher; host-implemented commands (§3) run against a test host.
  These are the envelope's regression net, and they double as
  documentation by example.
- **Property tests** (proptest — the kernel's own idiom, rule 3). For
  arbitrary command sequences: a refused transaction leaves the
  serialized document byte-identical; every successful *model-mutating*
  envelope (§6.4 — history commands pop entries rather than add them,
  and sit outside this property) adds exactly one undo entry, and
  undoing it either restores the prior serialized bytes or fails typed,
  with the documented kernel gaps (today: the deferred `UnbuildPushPull`
  case) enumerated as an explicit, reviewed allowlist mirroring the
  kernel fuzz harnesses' posture; derived points resolve to the kernel's
  own exact values; ambiguous locators always refuse, never guess.
- **Registry completeness.** Every declared command carries parameter
  and result schemas, a summary, and a refusal inventory; the generated
  artifacts (MCP tool definitions, the TypeScript SDK, the published
  reference) regenerate byte-identically in CI, so drift between
  registry and artifact is a build failure rather than a review hazard.
- **Determinism.** A script of envelopes replayed headlessly produces a
  byte-identical `.hew` — the API inherits, and must never break, the
  kernel's replay guarantee (§2).

Sequencing: a command's registry declaration and its conformance tests
merge first, with the not-yet-implemented command dispatching a distinct
`unimplemented` refusal and its tests explicitly marked as gated on it.
Implementing the command consists of removing that marker and making the
pre-written tests pass unmodified. The marked set is therefore a
visible burn-down checklist, and a command is "done" precisely when its
tests say so. Where this document and the suite are ever found to
disagree, that is a specification bug fixed in both, in the open — never
resolved silently in whichever direction is convenient.



## 15. What the API is not

- **Not the internal recording format.** The kernel's op/replay enums
  remain private and volatile; `crates/api` owns the mapping.
- **Not a mesh API.** No vertex/face assembly surface exists at any
  profile, by design position (§2), not by omission.
- **Not REST, not GraphQL, not a broker.** Operations with ordering,
  transactions, and typed refusals are RPC-shaped; introspection comes
  from the registry; single-writer needs no middleware (§1).
- **Not the workbench API.** Registering tools, panels, and UI — what
  plugins and overlays need beyond model access — is a separate, future
  surface. This API is the model half; the workbench half will be
  specified with the plugin system and will use this one for every model
  mutation it makes.

## 16. Future directions (non-normative)

Named here only to record that the design leaves room for them: the plugin
transport and manifest-scoped grants (§10, §11.4); overlay/workbench
surfaces built atop the same bus; a binary encoding negotiated at
handshake for an out-of-process kernel (§4.3); and the eventual migration
of Hew's own UI onto the bus as its first and most privileged client
(§11.1).
