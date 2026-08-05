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
> what the user has open. `hew.view.snapshot` likewise refuses live (no
> viewport handle reaches `crates/api` for it to render through yet), but
> `hew.view.camera`, `hew.view.zoom_extents`, and `hew.view.units` — which
> need no bytes back, only an effect — ARE wired end to end: `LiveHost`
> records the requested effect and answers success, `crates/wasm-api`
> hands it to the webview's live bridge (`app/src/api/liveBridge.ts`) as a
> directive read out right after dispatch, and the bridge applies it
> through the Viewport's existing camera calls or
> `app/src/settings/units.ts`'s own setter — the same code paths the
> Camera menu and Settings window already use. So `--live` today is real
> for the kernel-served command surface (sketch/solid/structure/entity/
> style/attrs/history — the bulk of the protocol), for `save`/`export`,
> for these three view/display effects, and honestly refuses the
> remaining host-effect commands that need filesystem or rendering access
> this sandbox does not have. Sections marked *Reserved* remain
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
foreign mesh data. This is the load-bearing decision: it is what makes
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
(ROADMAP.md, Reliability). The API surfaces those refusals verbatim as
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

**Profiles are views, not protocols.** A profile is a named subset of the
command registry a host grants a connection (§10). Every profile speaks
the identical envelope; granting more capability never changes the
protocol, only the answer to "which methods may this connection call."

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
| `crates/api` | Command registry, envelope types, dispatch onto the kernel, transaction execution, profile enforcement, schema publication | `kernel`, `inference`, `tessellate` |
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
 {"kind": "object", "name": "Leg", "watertight": true,
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
 "params": {"protocol": 1, "token": "…", "client": {"name": "hew-cli", "version": "0.6.0"},
            "encodings": ["json"]}}
```

The response reports the host's protocol version, application identity,
the granted profile, the negotiated encoding, and the open documents:

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
with several windows), the connection then binds to one with
`hew.doc.attach`; every subsequent envelope on that connection addresses
the attached document. A connection is attached to at most one document
at a time. Document ids are unique for the host's lifetime and never
reused, so a stale attach can never silently target the wrong document;
closing a document detaches every attached connection, whose subsequent
requests answer `-32002`. In hosts that honor `hew.doc.new`/`open`, the
issuing connection is attached to the resulting document automatically —
no separate `attach` round-trip.

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
  (already exhaustively maintained — ROADMAP.md, Reliability). New refusal
  names may appear in any release; clients must treat an unrecognized
  refusal as "refused, document untouched" and read the explanation.
- **Internal faults** — code `-32003`. A kernel invariant failed (the
  always-on validator caught a would-be corruption and rolled back). The
  document is untouched; the condition is a bug to report, not to handle.

### 4.5 Notifications (Reserved)

Server→client notifications share the envelope (no `id`) and are reserved
at protocol 1: the frame shape is fixed now so no transport ever needs a
second channel, but no subscription commands ship in the first release.
The planned inventory — `hew.event.document_changed`,
`hew.event.selection_changed`, `hew.event.context_changed`,
`hew.event.document_saved`, with a `hew.meta.subscribe` command — is
non-normative until specified.

## 5. Identity and addressing

### 5.1 Entity identifiers

Objects, Sketches, Groups, Component definitions, Instances, Guides,
Materials, and Tags are addressed by opaque string identifiers
(`"obj_5f3a"` above is illustrative; clients must not parse ids).

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
its regions and curves, and answers a query on an edge id directly;
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

Guides are addressable entities but not box-shaped: a guide *line* is
infinite — an origin and a direction, not a finite segment — so `bbox`
anchors on one refuse typed rather than fabricating a meaningless
corner. Its origin and direction read back via `hew.query.entity`, and
intersection-based alignment (the crossing of two guides) is client-side
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
  their summaries.
- **Read-only** commands — `hew.query.*`, `hew.meta.*`, and
  `hew.attr.get` — are legal anywhere: standalone (adding no undo
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

## 7. Command surface

The namespaces below and the semantics stated for them are normative; the
per-command schemas are the registry's (§9). Tiers: **Required** commands
exist at 1.0 and are the minimum for the agent-modeling vision; **Standard**
commands exist at 1.0 for tool parity; **Reserved** namespaces are designed
for, not shipped.

| Namespace | Contents | Tier |
|---|---|---|
| `hew.meta` | `hello`, `capabilities`, `documents` | Required |
| `hew.doc` | `attach`, `transact`, `new`, `open`, `save`, `export` (STL/3MF/glTF), `import` (foreign formats) | Required (`import` Standard) |
| `hew.query` | `scene` (tree + per-entity summaries), `entity`, `faces` (planes, areas, centroids, boundary loops), `raycast`, `measure`, `resolve` (§5.3), `context` (the open frame stack) | Required |
| `hew.sketch` | `draw_line`, `draw_rect`, `draw_circle`, `draw_arc`, `draw_polygon`, `offset` | Required |
| `hew.solid` | `extrude` (region → new Object), `push_pull` (face of a solid), `union`, `subtract`, `intersect`, `slice`, `follow_me` | Required (`follow_me` Standard) |
| `hew.entity` | `rename`, `delete`, `move` (with copy/array), `rotate`, `scale` | Required |
| `hew.context` | `enter`, `exit` | Required |
| `hew.group` | `create`, `explode` | Required |
| `hew.component` | `create`, `place`, `make_unique`, `explode` | Standard |
| `hew.material` | `create` (color or texture), `paint`, `set_default`, `set_opacity` | Standard |
| `hew.tag` | `create`, `assign`, `set_visible`, `delete` | Standard |
| `hew.guide` | `line`, `point`, `angular`, `clear` | Standard |
| `hew.attr` | `get`, `set`, `delete` (§8) | Required |
| `hew.history` | `undo`, `redo`, `status` (depth; top entry's label and origin) | Required |
| `hew.view` | `snapshot` (render the attached document to PNG, headless or live), `camera` (set the live viewport's camera), `zoom_extents` (frame all visible geometry), `units` (set the app's displayed length-unit format) | Standard (`core` grants `snapshot` specifically; `camera`/`zoom_extents`/`units` stay `app`-only — live-host-only effects) |
| `hew.annotate` | dimensions, leader text | Reserved |
| `hew.event` | notifications + `subscribe` | Reserved (§4.5) |

Semantics notes, normative:

- Drawing commands take a **plane spec**: `{"ground": true}`, an explicit
  `{origin, normal[, x_axis]}` frame, `{"face": <locator>}` (draw on a
  solid's face — the face-imprint path), or `{"sketch": id}` (extend an
  existing sketch). Points are 3D world coordinates and must lie on the
  plane within kernel tolerance; off-plane input is refused typed, never
  projected silently. Any point parameter accepts a derived-point locator
  (§5.3) in place of coordinates.
- `hew.sketch.draw_arc` takes a `close`: `"open"` (default, a bare arc),
  `"pie"` (closed wedge — two radii to the center), or `"segment"`
  (closed circular segment — the chord) — the API counterpart of the UI
  Arc tool's Alt-cycled completion modes. `"pie"`/`"segment"` commit a
  closed profile exactly like `draw_rect`/`draw_circle`: a region in
  plane/sketch mode, a `SplitFaceInner` loop imprint in face mode.
  Refused typed when combined with a full-turn sweep, which is already a
  closed loop (the circle special case).
  `"pie"`/`"segment"` additionally need at least 2 chords: a single chord
  can't form a non-degenerate closed loop (a one-chord `"segment"`'s
  closing edge would retrace the same chord — no region at all; a
  one-chord `"pie"` risks a collinear, zero-area wedge at some sweeps,
  sharpest at a half turn). `segments` below that floor is refused
  static (`-32602`); the proportional default is floored at 2 as well,
  so a small sweep never silently falls into the same degenerate case.
  Face mode additionally validates every loop vertex — including
  `"pie"`'s caller-supplied center — strictly inside the target face
  before cutting, refused `loop_not_strictly_inside` otherwise; this is
  stricter than the bare containment test alone, which is not reliable
  for a point sitting exactly on the face's own boundary.
- In headless hosts `hew.doc.new`/`open` establish the working document.
  A live host at 1.0 keeps document lifecycle user-driven — the user
  creates or opens documents and clients `attach`; whether a live host
  additionally honors `new`/`open` (opening a fresh window, say) is
  host-defined and advertised via capabilities.
- `hew.doc.import` merges a foreign-format file (STL, glTF/GLB, COLLADA,
  `.skp`) into the attached document as new entities, through the same
  importer crates and shared healing pipeline the UI uses — it is not
  `open`, which loads a `.hew` document. Import options mirror the UI's
  import-time questions as explicit parameters; notably STL, which
  carries no units, refuses typed unless `units` is given — there is no
  one to prompt.
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
- The transform commands are contract-shaped as: `move` takes a
  translation vector or a from→to point pair; `rotate` takes a pivot
  point, an axis direction, and an angle; `scale` takes an anchor point
  and per-axis factors — every point-typed parameter accepting a
  derived-point locator (§5.3). `move`'s array form carries the UI's
  semantics (`count` copies at the committed step, multiplying or
  dividing the distance); exact schemas live in the registry.
- Convenience/primitive commands (`create_box`, …) are deliberately
  absent from 1.0. If they are added later they will be composites defined
  over these core commands, adding no new kernel semantics; whether they
  arrive as core sugar or as a first-party plugin is an open product
  decision this spec does not preempt.
- `hew.view.snapshot` exists so a client can *see* the model rather than
  only query it — for an agent, the difference between knowing what is
  true and seeing what it built. A headless host renders it through a
  deterministic software rasterizer (no GPU, no viewport, bytes in and
  bytes out); a live host may render through its actual viewport instead
  — either way the command takes an explicit camera or a named standard
  view (`camera` and `view` mutually exclusive; giving neither renders the
  document's saved working camera if it has one, else a fitted isometric
  view) and returns PNG bytes base64-encoded by default, or writes them
  to an explicit `path` instead (below). `include_ids: true` also
  returns a per-pixel id-buffer (`u16` little-endian, 0 = background) and
  the palette of public ids it indexes, so a caller can ask "what object
  is at pixel (x, y)" machine-readably.
- `hew.history.undo`/`redo` act on the shared document history — the
  topmost entry, whoever authored it. Because that is a blunt instrument
  in a `--live` session, `undo` accepts an optional `expected_label`,
  refusing typed when the top entry isn't the named one, and
  `hew.history.status` reports the depth and the top entry's label and
  origin — so an agent verifies the top entry is its own before popping,
  rather than blindly undoing the user's last edit. History commands are
  solitary (§6.4) and sit outside the one-envelope-one-undo accounting.
  Undo through the API is subject to the same replay contract as the
  UI's (DEVELOPMENT.md rule 9): it either restores exactly or fails
  typed — the deferred `UnbuildPushPull` case (ROADMAP.md) is today's
  one documented typed-failure gap — and it never corrupts.
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
  itself stays inline either way, since it is small.
- `hew.entity.move` with `copy` over a Sketch selection refuses typed at
  1.0, whatever the count: the UI's sketch copy is tool-layer replay
  through the sticky rules, and the kernel-side sketch duplicate op does
  not exist yet (ROADMAP.md). The refusal is named in `move`'s registry
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
  has no display preference to set and refuses `host_capability_missing`.

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
  Enforcement sits at the API boundary rather than in the kernel — the
  reservation exists so first-party code can claim those namespaces, so
  the kernel and the UI keep writing them freely. Reads are
  unrestricted; `hew.attr.get` returns first-party dictionaries like any
  other.
- Values are arbitrary JSON (finite numbers only), read and written at
  key granularity: `hew.attr.set {target, ns, key, value}`,
  `hew.attr.get {target, ns?}`, `hew.attr.delete {target, ns, key?}`.
- Attribute writes are ordinary mutations: transactional, undoable, and
  serialized in `.hew`. Copy and instancing semantics follow the entity
  (a deep-copied group copies its dictionaries; instances carry their
  own, definitions theirs).
- Dictionaries survive round-trip through clients that do not understand
  them — that is their entire contract.

This is the extension point future plugins and overlays persist through —
a wall's BIM type, a plugin's per-object state — and it ships in 1.0
precisely because it is a native-file-format change (a manifest schema
addition), and format decisions are vastly cheaper before the 1.0 format
freezes than after. The wire-level `.hew` encoding is specified in
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

## 10. Profiles

A profile is the set of methods a host grants a connection, enforced at
the dispatcher (`-32001` outside it). Enforcement reaches **inside**
transactions: every command of a `hew.doc.transact` is checked against
the connection's profile statically, before any command executes, and a
violation is `-32001` carrying the offending index. `hew.doc.transact`
itself grants nothing — it is a container, not a capability. This is
pinned at protocol 1 because it cannot be retrofitted additively (§9),
and the entire future of scoped plugin grants depends on it. Protocol 1
defines two profiles:

- **`core`** — everything headless-safe: the full command surface except
  `hew.view.*`, with one deliberate exception — `hew.view.snapshot` has a
  headless render path (a software rasterizer, no GPU or live viewport
  required) and is granted here too. This is what `hew-cli` grants its own
  embedded kernel and the ceiling for what a future plugin's manifest can
  request.
- **`app`** — `core` plus the commands that only make sense against a
  live application session: `hew.view.camera`, `hew.view.zoom_extents`,
  and `hew.view.units` today (a future `hew.view.*` addition: selection).
  Granted by the desktop app to authenticated local connections.

A profile is a *maximum* a host may narrow: a live host that keeps
document lifecycle user-driven simply withholds `hew.doc.new`/`open`
from its `app` grant. `hew.meta.capabilities` is the sole authority for
what a connection actually holds, and any registry command outside the
grant answers `-32001` — never `-32601`, which is reserved for methods
the registry has never heard of.

Profiles are the mechanism by which the plugin system will later express
scoped capability grants ("this plugin may read but not mutate") without
touching the protocol: a plugin profile is just a smaller view of the same
registry.

## 11. Transports

### 11.1 In-process

The UI and headless tools call `dispatch` directly; Rust and wasm callers
use the envelope types natively with no serialization. The UI is not
required to route through the public API at 1.0 — but the standing
direction is that the UI never performs a *model mutation* the bus cannot
express, so that a future migration of the UI onto the bus (the "first
overlay" end-state) is never structurally foreclosed.

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

## 13. MCP mapping

The MCP server does not expose one tool per command — eighty near-flat
tools would bury an agent in choices while stripping the transaction
semantics that make multi-step work atomic. It serves a small set of
chunky tools, all generated from the registry:

- `hew_capabilities` — the introspection surface (§9), so an agent can
  learn the exact command schemas at run time.
- `hew_transact` — the workhorse: a full transaction envelope (§6),
  returning per-command results or the typed refusal.
- `hew_query` — the read surface: any read-only command (§6.4 —
  `hew.query.*`, `hew.meta.*`, `hew.attr.get`) and its params.
- `hew_describe_scene` — `hew.query.scene` presented as a structured
  summary tuned for reasoning over (names, kinds, bounding boxes,
  watertightness, tree shape).
- `hew_snapshot` — `hew.view.snapshot`, so the agent can look at what it
  built, not merely query it. Present headless: `hew.view.snapshot` now
  has a headless render path (a software rasterizer), and `core` grants it
  specifically, so a headless MCP session lists this tool too.

The tool list is generated from the connection's granted profile: every
profile today grants `hew_snapshot`, since core's one carve-out is exactly
this command; a future `hew.view.*` addition that stays `app`-only would
be the next tool a headless session lacks. An agent never sees a tool it
cannot call.

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

Named here only to record that the design leaves room for them: event
subscriptions over the reserved notification frames (§4.5); the plugin
transport and manifest-scoped profiles (§10, §11.4); overlay/workbench
surfaces built atop the same bus; a binary encoding negotiated at
handshake for an out-of-process kernel (§4.3); and the eventual migration
of Hew's own UI onto the bus as its first and most privileged client
(§11.1).
