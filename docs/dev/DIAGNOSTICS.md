# Hew — Diagnostics

Hew ships three diagnostic facilities: a structured **debug log**, a
**session recording/replay** mechanism, and an automatic **crash-reproducer
bundle** that combines both. Together they let a bug report carry enough
state to reproduce the problem deterministically, instead of a description
of what was on screen.

This doc is for anyone filing or triaging a Hew bug. It specifies the
on-disk formats normatively — so a bug report can be inspected or replayed
by anyone, without proprietary tooling — and describes how to enable, read,
and attach each facility.

## 1. Debug log

### What it captures

The kernel (compiled to WASM) and the UI each emit structured events into one
merged, time-ordered stream:

- **Kernel events** — one record per `tracing` event raised by the kernel and
  inference crates: the start of each document-mutating operation (name +
  scalar parameters), the resulting canonical state digest after it commits (on a document of more than 50,000 faces, its face count instead),
  and (at a finer level) inference/snap resolution during pointer movement.
- **UI events** — the application's own info/warning/error log, bridged into
  the same stream.

Both halves share a monotonic sequence number and a per-gesture correlation
id, so the merged log can be filtered down to the exact user action (a single
drag, a single click) that a kernel record and its surrounding UI events
belong to.

### Where it's written

The log always accumulates in an in-memory ring buffer (default capacity
50,000 records; oldest records drop first) regardless of platform. Writing it
to disk is opt-in, via **Debug Mode** (Settings → Debug → "Enable Debug
Mode"):

- **Desktop**: a rolling file, `diagnostic.log`, in the platform's standard
  application log directory. When it exceeds 10 MiB it is rotated to
  `diagnostic.1.log` (replacing any previous backup) and a fresh
  `diagnostic.log` is started — one backup is kept.
- **Web**: there is no addressable filesystem, so the ring buffer is the
  source of truth. The same Settings pane offers a "Download diagnostic log"
  button that saves the current buffer as an NDJSON file.

Debug Mode also enables low-level input recording and kernel "torture mode"
(extra validation and a re-tessellation self-check after every operation) —
see and.

### Format (normative)

The log is **newline-delimited JSON** — one `LogRecord` object per line, no
enclosing array. Field order is not significant, but a conforming producer
emits object keys sorted lexicographically. Non-finite floating-point values
(`NaN`, `Infinity`) are encoded as their Rust `Display` string, since JSON has
no representation for them.

```jsonc
{
  "seq": 41,                 // u64. Monotonic across the session; strictly
                              //   increasing, never reused.
  "corr": 7,                 // u64 | null. Correlation id of the user
                              //   gesture this event belongs to, or null
                              //   outside any gesture.
  "level": "INFO",           // "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR"
  "target": "kernel::op",    // emitting module path / channel, see below
  "fields": {                // structured key/values; well-known keys below
    "op": "extrude_region",
    "distance": 2.0
  }
}
```

**Targets (channels):**

| `target` | Level | When | Key `fields` |
|---|---|---|---|
| `kernel::op` | INFO | start of each document-mutating operation | `op` (name) + scalar parameters (`distance`, `boolean_op`, `node`, …) |
| `kernel::cmd` | INFO | after each committed mutation | `state_hash` (canonical digest after the op), `objects` (count touched); on a document of more than 50,000 faces, `state_hash_skipped_faces` (the face count) in place of the digest |
| `inference::resolve` | TRACE | every snap query (per pointer move) | `candidates` (count), `winner` (snap kind, or `None`) |

A single user gesture therefore brackets as one `kernel::op` record (the
command and its parameters) followed by one `kernel::cmd` record (its
effect on the canonical state), sharing a `corr` value.

Well-known `fields` keys:

- `op` — command name, matching the underlying document method.
- `state_hash` — the document's canonical state digest after the operation.
  Two runs that are supposed to be identical (e.g. original vs. replay) can be
  compared op-by-op by walking matching `state_hash` values, which pinpoints
  the exact operation at which they diverge. The digest is a full
  serialization of the document, so on one of more than 50,000 faces the
  record carries `state_hash_skipped_faces` instead; the recording's
  `golden_hash` at stop still checks the whole replay.
  The serialization hashed carries a fixed `app_version` and a fixed
  manifest `format_version` (22), not the writer's own, so neither a
  release nor a format bump that adds an optional field moves the digest
  of a document that does not use the field; hashes frozen into replay
  fixtures and recordings stay valid.
- `message` — free-text, when present.
- `winner`, `candidates` — inference/snapping diagnostics.

### Capture level

The capture level is a runtime threshold; events below it are dropped at the
source rather than merely hidden. Default is **INFO**, which captures the
operation/command stream but drops the per-pointer-move `inference::resolve`
firehose. Raise it to `TRACE` to investigate a specific bad snap.

## 2. Session recording & replay

### What a recording contains

A recording is the ordered stream of **committed** scene mutations, captured
as typed, replayable calls — not raw video or screenshots. Replaying the
calls into a fresh document reproduces the session exactly, including the
internal object identifiers the kernel assigned, because the kernel's
operations are deterministic (the same call sequence always performs the
same insert/remove sequence). This means a recording needs no id-remapping
step to replay: the recorded identifiers are valid verbatim.

Only mutations that succeeded are recorded — a rejected or failed operation
never appears, so a recording can't reproduce an operation that never
happened.

Mutations that arrive over the live API (an agent or script driving the
open document, HEW_API.md §11.2) are recorded too, as `api_dispatch`
entries holding the request verbatim. A session co-authored by a person
and an agent therefore replays whole; without this the agent's half would
be invisible, and a bug report built from such a session would silently
reconstruct a different document than the one that broke. They are stored
at API altitude rather than expanded into the kernel calls they perform:
the API is a versioned contract, so a recorded envelope survives kernel
refactors that would invalidate a kernel-shaped capture, and one envelope
may be a whole transaction whose grouping into a single undo entry is part
of what a faithful reproducer has to preserve. Read-only API traffic — an
agent polling the scene — is not recorded.

A forwarded plugin command (docs/agents/PLUGINS.md "Compounds") records
differently, because no plugin runs at replay time. The forwarded call
itself is never recorded. While its compound is open, every call that
would be recorded (the plugin's own envelopes, and anything that records
itself, such as an import) is held in a buffer instead; when the compound
ends, the buffered calls are recorded as one `plugin_compound` entry, and
when it aborts they are dropped, so an aborted command leaves no trace. A
plugin command that calls another nests: the inner command's
`plugin_compound` is one of the outer's buffered calls. A crash while a
compound is open still reproduces: the crash snapshot carries the
buffered calls after the recorded ones.

Adding this entry did not bump the format version, following the same
rule every other added call follows: a recording that never used the new
entry replays on an older build unchanged, and one that does fails to
parse there loudly rather than replaying into a different document. The
version marks a breaking change to the envelope's shape, and replay
rejects anything that does not match it exactly — so bumping for an
addition would strand every previously frozen reproducer.

High-level command recording runs continuously in the background for every
open document; it is cheap because operations occur at user-gesture
frequency, not per frame. A second, optional layer — enabled together with
the debug log by Debug Mode — additionally captures raw UI input (pointer
moves, clicks, camera state, key presses) as a sibling array in the same
file. Replaying that layer drives the entire interactive stack, including
inference and snapping, which is what's needed to reproduce a bug that lives
in the UI layer rather than the kernel.

### Format (normative)

A recording is one JSON object.

```jsonc
{
  "version": 2,              // u32, format version. Bump on breaking changes.
  "calls": [                 // committed mutations, in application order
    { "method": "begin_ground_sketch" },
    { "method": "sketch_add_segment", "sketch": 1, "a": [0,0,0], "b": [1,0,0] },
    { "method": "sketch_add_segment", "sketch": 1, "a": [1,0,0], "b": [1,1,0] },
    { "method": "extrude_region", "sketch": 1, "region": 1, "distance": 2.0 },
    { "method": "boolean", "op": 0, "a": 1, "b": 2 },
    { "method": "slice_object", "object": 3, "plane": [0,0,1, 0,0,1] },
    { "method": "transform_object", "object": 3,
      "affine": [1,0,0,0.5, 0,1,0,0, 0,0,1,0] },
    { "method": "delete_node", "kind": 0, "id": 4 },
    { "method": "api_dispatch",
      "frame": "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"hew.solid.extrude\",\"params\":{}}" },
    { "method": "plugin_compound", "plugin": "com.example.parts",
      "label": "Cut list", "grant": { "ceiling": "core", "permissions": ["model.write"],
        "plugin": "com.example.parts", "app": null, "document": null },
      "calls": [ { "method": "api_dispatch", "frame": "…" } ] }
  ],
  "golden_hash": 5678,       // u64. Document state digest after the last
                             //   call — the replay oracle.
  "input": [ /* optional, see below */ ]
}
```

Each entry in `calls` is a tagged object: `method` selects the call, and the
remaining fields are its arguments. `sketch`/`region`/`object`/`id` are the
literal internal handles from the recording session. Drawing on a face is
recorded as `draft_*` calls; recordings made before the drafting engine,
which held the retired face calls (`split_face_inner`, `split_face`,
`merge_faces`, `dissolve_imprint`, `transform_imprint`, and the rest), no
longer replay.

| `method` | Arguments | Effect |
|---|---|---|
| `api_dispatch` | `frame` | re-dispatch one document-mutating live-API request, verbatim (HEW_API.md §11.2) |
| `plugin_compound` | `plugin`, `label`, `grant`, `calls[]` | one forwarded plugin command's compound: open a connection holding `grant` (identity `plugin:<plugin>`), open a compound labeled `label` with that origin, replay `calls` in order (their `api_dispatch` frames on that connection; a nested `plugin_compound` the same way, recursively), end the compound, and close the connection. Lands as one undo entry, as it did live. A call refused inside it fails the replay |
| `begin_ground_sketch` | — | begin a ground-plane sketch |
| `begin_sketch_on_plane` | `px`,`py`,`pz`,`nx`,`ny`,`nz` | begin a sketch on an arbitrary plane (point + normal, normalized) — the idle plane lock's first-click mint (sketches-on-any-plane design §5) |
| `sketch_begin_gesture` / `sketch_end_gesture` | `sketch` | bracket one multi-segment commit as a single undo step |
| `sketch_cancel_gesture` | — | abandon an open gesture bracket |
| `sketch_begin_curve` / `sketch_end_curve` | `sketch` | bracket the segments of one drawn arc/circle as a curve chain |
| `sketch_begin_curve_with` | `sketch`, `center[3]`, `radius` | curve bracket carrying the chain's analytic circle |
| `sketch_add_segment` | `sketch`, `a[3]`, `b[3]` | add a segment to a sketch |
| `sketch_remove_edge` | `sketch`, `edge` | remove one sketch edge (the eraser's commit) |
| `sketch_begin_gesture` / `sketch_end_gesture` | `sketch` | bracket one drawing gesture (one undo step) |
| `sketch_cancel_gesture` | — | drop the open gesture without recording |
| `sketch_begin_curve` / `sketch_end_curve` | `sketch` | bracket segments into one curve chain |
| `sketch_begin_curve_with` | `sketch`, `center[3]`, `radius` | curve bracket carrying the chain's analytic circle |
| `extrude_region` | `sketch`, `region`, `distance` | extrude a closed profile into an Object |
| `follow_me_along_edges` | `sketch`, `region`, `path_sketch`, `path_edges[]` | sweep a closed profile along a chain of sketch edges into an Object |
| `follow_me_around_face` | `sketch`, `region`, `path_object`, `path_face` | sweep a closed profile around a solid face's outer boundary into an Object |
| `boolean` | `op` (0=union, 1=subtract, 2=intersect), `a`, `b` | combine two Objects |
| `boolean_nodes` | `op` (as `boolean`), `a_kind`/`a`, `b_kind`/`b` (kind 0=object, 1=group) | combine two tree nodes — plain solids or whole groups; the route every UI boolean command takes |
| `boolean_through_placement` | `path[]` (instances, outermost first), `host` (omitted: the definition's only part), `tool`, `op` (as `boolean`; only 1=subtract runs, union and intersect refuse `BooleanOperandHasInstance`) | subtract a world tool Object from a definition member seen through a placement; every placement shows the result |
| `group_nodes` | `kinds[]`/`ids[]` (parallel node lists, kind as `delete_node`) | form a merge group |
| `duplicate_node` | `kind`, `id`, `affine[12]` (row-major 3×4) | deep-copy a node (Move+Alt) at the affine offset |
| `slice_object` | `object`, `plane[6]` (`[px,py,pz,nx,ny,nz]`) | slice an Object by a plane |
| `draft_begin_plane` / `draft_begin_face` / `draft_begin_sketch` / `draft_begin_ctx` | `draft` (the handle replay re-mints), the plane (`px..nz`), `object`/`face`, `sketch`, or `ctx` (JSON), `label` | open a drafting gesture on a plane (draw on the ground or on a face); `_in_instance` forms also carry `instance` and take world coordinates |
| `draft_begin_detached` | `draft`, `ctx` (JSON: a `plane` or `face` context), `label` | open a drafting gesture that draws one new detached shape (it joins nothing on the plane) |
| `draft_add_segment` | `draft`, `a[3]`, `b[3]` | add a segment to the open draft's drawing |
| `draft_begin_curve` / `draft_begin_curve_with` / `draft_begin_polygon_with` / `draft_end_curve` | `draft`, plus `center[3]`, `radius` for the `_with` forms | bracket segments into one curve chain (a plain run such as a flat arc), circle, or regular polygon |
| `draft_remove_edges` / `draft_transform_edges` / `draft_copy_edges` | `draft`, `keys[]` (edge keys), `affine[12]` | erase, move, or copy drawing by edge key |
| `draft_move_vertex` / `draft_offset_region` / `draft_refacet_curve` | `draft` plus the point, region key and distance, or curve key and segment count | the vertex, offset, and re-facet edits |
| `draft_end` / `draft_cancel` | `draft` | lower the draft as one undo step / drop it |
| `history_bracket_open` / `history_bracket_commit` / `history_bracket_abort` | `label` (commit) | bracket a multi-edit gesture (a selection transform that moves objects and drawing) into one undo step, or restore the document as it was at open |
| `push_pull` | `object`, `face`, `distance` | push/pull a solid face; the kernel re-derives the routing (translate, whole-wall radial offset, boss/recess, or through-cut) on replay |
| `transform_object` | `object`, `affine[12]` (row-major 3×4) | apply an affine transform |
| `transform_selection` | `kinds[]`/`ids[]` (parallel node lists), `sketches[]`, `affine[12]` | transform a whole multi-selection as one undo step |
| `delete_node` | `kind` (0=object, 1=group, 2=instance), `id` | delete a node |
| `duplicate_node` | `kind`, `id`, `affine[12]` | Move+copy of one node: a deep clone placed at the offset |
| `duplicate_selection_array` | `kinds[]`/`ids[]` (parallel node lists), `affine[12]`, `count` (1–1000; an out-of-range count fails the replay typed rather than hanging), `linked` (optional, written only when `true`) | array copy: every listed node cloned `count` times along the step, one undo step. With `linked`, each Object or Group is first wrapped as its own linked set and the copies are placements of it; a recording without the field replays as independent copies |
| `duplicate_selection_placements` | `kinds[]`/`ids[]` (parallel node lists), `affines[]` (12 floats per placement, row-major 3x4, concatenated; 1–1000 placements, otherwise the replay fails typed), `linked` (optional, written only when `true`) | grid array copy: every listed node cloned once per placement, one undo step, placement-major. With `linked`, each Object or Group is first wrapped as its own linked set and the copies are placements of it; a recording without the field replays as independent copies |
| `scene_undo` / `scene_redo` | — | document-level undo/redo (recorded only when it succeeded) |
| `transform_sketch` | `sketch`, `affine[12]` | bake an affine into a free-standing sketch |
| `transform_sketch_island` | `sketch`, `island`, `affine[12]` | rigidly move one sketch island |
| `copy_sketch_islands` | `sketch`, `islands[]`, `affine[12]` | copy a sketch's islands onto one new sketch off-plane (Move+Alt off the plane), one undo step |
| `move_sketch_vertex` | `sketch`, `vertex`, `p[3]` | drag one sketch vertex |
| `delete_sketch` | `sketch` | delete (hide) one free-standing sketch |
| `duplicate_node` | `kind`, `id`, `affine[12]` | deep-clone a node at an offset (Move+Option copy) |
| `group_nodes` | `kinds[]`, `ids[]` | group sibling nodes into a merge group |
| `ungroup` | `group` | dissolve a group |
| `transform_group` | `group`, `affine[12]` | bake an affine into a group's leaves |
| `make_component` | `kinds[]`, `ids[]` | fold a selection into a shared definition + instance |
| `place_instance` | `component`, `affine[12]` | stamp another instance of a definition |
| `transform_instance` | `instance`, `affine[12]` | compose an affine into an instance's pose |
| `explode_instance` | `instance` | bake an instance into independent world objects |
| `make_unique` | `instance` | detach an instance onto a private definition copy |
| `make_unique_by_path` | `path[]`, `cascade`, `deep` | fork the placement at the end of an instance path (outermost first) onto a private definition copy; `cascade` forks every definition on the path so only that placement diverges, `deep` also forks the definitions nested inside the leaf's. One undo entry |
| `wrap_linked` | `kind` (0=object, 1=group), `id` | wrap one Object or Group as a linked set: a definition carrying `hew` / `linked` with the node as its only member, placed once in the node's spot. One undo entry |
| `set_linked` | `component`, `linked` (only `false` replays; `true` is refused) | remove `hew` / `linked` from a linked set, so it reads as a named component. One undo entry |
| `push_pull_in_component` | `component`, `object`, `face`, `distance` | push/pull a face inside a component definition |
| `set_node_name` | `kind`, `id`, `name` (string or null) | rename a node / clear its name |
| `add_node_tag` / `remove_node_tag` | `kind`, `id`, `path[]` (segments) | assign / unassign a tag path on a node |
| `set_tag_hidden` | `path` (`/`-joined), `hidden` | set a tag's hidden-by-default flag (persisted view state) |
| `delete_tag` | `path` (`/`-joined) | delete a tag — and its sub-tags — everywhere |
| `set_node_user_hidden` | `kind`, `id`, `hidden` | set a node's persisted user-hidden flag |
| `add_material` | `name`, `r`, `g`, `b`, `a` | add a solid-color palette material |
| `add_texture_material` | `name`, `r`, `g`, `b`, `a`, `image[]`, `format` (0=PNG, 1=JPEG), `world_w`, `world_h` | add a textured material (embeds the encoded image bytes) |
| `set_material_alpha` | `material`, `alpha` | set a palette material's opacity |
| `paint_face` | `object`, `face`, `material` (`u64::MAX` = unpaint) | paint one face |
| `set_object_material` | `object`, `material` (`u64::MAX` = clear) | set an object's base material |
| `set_instance_material` | `instance`, `material` (`u64::MAX` = clear) | set a component placement's own material |
| `set_definition_material` | `component`, `material` (`u64::MAX` = clear) | set the base material of every object a definition owns, as one undo entry |
| `add_guide_line` | `origin[3]`, `dir[3]` | add a construction line |
| `add_guide_point` | `p[3]` | add a construction point |
| `delete_guide` | `guide` | delete one construction guide |
| `delete_all_guides` | — | delete every visible guide in one undo step |
| `import_dae` | `bytes[]`, `images[]` (`{uri, bytes[], format}` objects) | additive COLLADA import (embeds the file + images) |
| `import_gltf` | `bytes[]` | additive glTF/GLB import (embeds the file) |
| `import_skp` | `bytes[]` | additive `.skp` import (embeds the file) |
| `import_stl` | `bytes[]`, `unit_scale`, `name` (string or null) | additive STL import (embeds the file); `unit_scale` is meters-per-STL-unit since STL carries no units of its own, and `name` is the file stem the Objects are named from (STL carries no names) — both embedded so replay reproduces the same geometry size and Object names |
| `load` | `bytes[]` | replace the whole document — a mid-session File ▸ Open/New (embeds the `.hew` bytes) |
| `set_pick_scope` / `clear_pick_scope` | `objects[]`, `instances[]`, `frame_sketches` (set only; empty lists are a scope, only `clear_pick_scope` clears; a recording without `frame_sketches` replays as before it existed: both lists empty clear, otherwise the session frame's drawing is in scope) | set or clear the editing scope the Shaded and X-ray picks favor (session-only view state, recorded so a replayed session ends with the same scope as the original; changes nothing the state hash covers, and the recording stores no pick results) |

**Coverage rule** (ratified with the recording audit): every `Scene` method
that pushes the document undo stack or mutates state included in
`Document::save` records itself. Anything less makes `scene_undo` /
`scene_redo` — which ARE recorded — replay against a differently-shaped
stack and silently reproduce a different session. Session-only state that
is neither undoable nor saved (inference hide sets, transient snap
segments, guide/axis snappability, torture mode) is deliberately not
recorded. The one exception is the editing scope (`set_pick_scope` /
`clear_pick_scope`), recorded so a replayed session ends up with the
same scope as the original, and a probe after replay sees the same scoped
picks. The recording proves no pick result (it stores none, and replay
issues no picks); the render mode is not recorded either, so a probe sets
the mode it means to test itself (the scope applies in Shaded and X-ray,
not in Wireframe).
Calls that carry file or image payloads (`import_*`, `load`,
`add_texture_material`) embed the raw bytes so a recording stays
self-contained and replayable from a fresh document; recordings that span
imports, mid-session opens, or texture additions are correspondingly
larger.

Coverage grows over time under a deliberately **additive posture**
(ratified with the true-curves work): adding a new `method` variant is a
non-breaking change and does NOT bump `version` — an old recording replays
on a new build unchanged, and a recording that uses a new method fails to
parse on an older build loudly (a typed error), never silently divergent.
`version` bumps (to 3 and beyond) are reserved for changes an old reader
would MISinterpret: renaming or re-typing an existing method's fields, or
changing a field's meaning. No such change is planned; existing recorded
fixtures stay valid as-is.

`u64` values (`golden_hash` and handle fields) can exceed a JSON number's
safe integer range in some languages — see the note under Replay below.

**Optional `input` array** — raw UI input that precedes kernel
interpretation, present only when captured (omitted entirely when empty):

```jsonc
{ "kind": "pointerdown", "seq": 0, "t": 0, "gesture": 1,
  "x": 410, "y": 250, "button": 0, "buttons": 1,
  "mods": { "shift": false, "alt": false, "ctrl": false, "meta": false } }
```

```jsonc
{ "kind": "camera", "seq": 1, "t": 3, "gesture": 1,
  "position": [8, 6, 8], "target": [0, 0, 0], "up": [0, 0, 1], "fovDeg": 45,
  "projection": "parallel", "orthoFrustumHeight": 12.4, "orthoZoom": 1 }
```

Discriminated by `kind`: `pointerdown` / `pointermove` / `pointerup` /
`camera` / `keydown`. `seq` is a global monotonic counter; `t` is milliseconds
since capture start (pacing, not identity); `gesture` is a per-gesture
correlation id that increments on each `pointerdown`. Pointer coordinates are
CSS pixels relative to the viewport canvas's top-left corner.

A `camera` event carries `position` / `target` / `up` / `fovDeg`, enough to
rebuild the camera and its orbit target under perspective. Two fields are
additive (present only in recordings captured after Parallel Projection
shipped; absent on an older recording implies perspective throughout, so
nothing pre-existing needs backfilling):

- `projection` (`"perspective" | "parallel"`, optional) — the active
  projection at capture time. `fovDeg` is always the PERSISTED perspective
  fov regardless of `projection` (`CameraRig` keeps it across toggles) — a
  `"parallel"` capture's `fovDeg` is what the camera would return to on a
  toggle back, not a meaningful "current" fov.
- `orthoFrustumHeight` / `orthoZoom` (optional numbers) — present only when
  `projection` is `"parallel"`; absent under perspective, where no ortho
  frustum is in play. The minimal pair needed to reconstruct the
  orthographic frustum: half-height = `orthoFrustumHeight / (2 *
  orthoZoom)`, half-width scales that by the replay viewport's own aspect
  ratio (frustum width itself is never recorded, only derived).

A recording with only `calls` (no `input`) is still a complete, valid
recording — the input layer is a diagnostic supplement, not a requirement.

### Replay

Replaying a recording re-issues each call verbatim into a **fresh**,
otherwise-empty document and produces a final state digest, which must equal
the recording's `golden_hash`. Any divergence — a different digest, or a call
that fails to apply — pinpoints a regression.

`u64` values (`golden_hash`, handles) can silently lose precision if parsed
through a JSON decoder that represents all numbers as IEEE-754 doubles (for
example, JavaScript's `JSON.parse`). Tooling that reads or writes recordings
in such a language should extract those fields as literal digit strings
(regex or equivalent) and convert them directly to an arbitrary-precision
integer type, never round-tripping through the language's default JSON
number type.

### Recording a session

High-level command recording is always active for the open document; no
action is required to start it. To additionally capture raw input (for
reproducing UI-level issues such as a bad snap or a tool-state race), enable
Debug Mode in Settings → Debug — this turns on input recording alongside the
debug log file.

The easiest way to obtain a recording for a bug you can trigger is to let
Hew capture it automatically: see §4 below. To capture one manually while
scripting or driving the app programmatically, the underlying calls are
`start_recording()`, `stop_recording()`, `is_recording()`,
`take_recording()` (returns the recording JSON above and clears the buffer),
and `peek_recording()` (the same JSON, but leaves the buffer in place — for
a caller that must not disturb a recording something else still needs, such
as the Report Bug bundle's peek alongside a later crash reproducer's take).

A kernel panic poisons the wasm instance, so every `Scene` method throws
afterwards — `take_recording()`/`peek_recording()` included. The panic hook
(`crates/wasm-api/src/lib.rs`) works around this by handing the recorder's
buffered calls to the page directly, from inside the panicking frame, as
`globalThis.__hewLastPanic = { at, message, recording }` (`recording` is
Recording JSON, or null if nothing had been recorded yet). Its `golden_hash`
is always `0` — a panicking frame can't hash the document — so treat it as a
reproducer, never a regression oracle. It's in-memory only (unlike the
sibling `localStorage['hew:lastPanic']` record, message-only, that survives
a reload): it exists purely so the current page session can still recover a
recording after the instance is poisoned, which is also why it doesn't
survive a reload itself.

A kernel can also stop without a panic: Rust's out-of-memory abort runs no
panic hook, and the page sees only a bare `WebAssembly.RuntimeError`
("unreachable") from the call, then "recursive use of an object" from every
call after it. The open and import paths catch that trap (`noteOpenFailure`
in `App.tsx`) and write the same capture from the page side
(`recordKernelStop` in `log/panicCapture.ts`): `__hewLastPanic` with a null
recording, the `localStorage` record, and the `hew:kernel-panic` event. The
message names the cause from the memory the kernel had reached — "Out of
memory opening X (137.0 MB); kernel memory 4095.9 MB" when it is at the
wasm32 ceiling, "Kernel trap opening X: …" otherwise — and the crash screen
shows that stop's own heading and explanation ("This model is too large for
Hew to open", with what to do) instead of the generic internal-error copy.
`recordKernelStop` stands down when the hook already captured a panic, so a
panic's recording is never overwritten by the page's thinner record.

### Replaying a recording

`tools/replay-runner` is a small, dependency-free Node program that loads the
kernel's WASM build and replays recording files against it, without needing
the UI, a browser, or a display server.

```sh
cd tools/replay-runner
npm run build      # builds the kernel's WASM bindings for Node (one-time,
                    #   or whenever the kernel changes)
npm test           # replays every fixture in fixtures/ and checks its digest
```

To check a specific recording (for example, one attached to a bug report)
against its own claimed digest:

```sh
node run.mjs --freeze <recording.json> <name>
```

This validates the recording's format version, replays it into a fresh
document, and compares the result against the recording's own `golden_hash`.
On a match, it saves the recording under `fixtures/<name>.json`, where it
becomes a permanent regression test: once a bug is fixed, the same recording
keeps failing the fix honest forever. On a mismatch it refuses and writes
nothing — a recording that doesn't reproduce its own claimed digest is
either non-deterministic or was captured against different code, and isn't
safe to commit as a fixture.

## 3. Determinism

Replay works because the kernel guarantees, for a given sequence of
committed calls: the same operations are applied in the same order, produce
the same internal object identifiers, and yield the same canonical state
digest. This determinism is what lets a recording be replayed with its
identifiers taken verbatim (no remapping table) and lets two runs be
compared mechanically by digest rather than by inspection. It also underlies
the recording format's use as a regression-test mechanism: a frozen
recording plus its digest is a fixture that either keeps passing or flags an
exact regression, with no flakiness from the comparison itself.

## 4. Reporting bugs

Hew automatically assembles a **reproducer bundle** whenever it hits an
unhandled error: it packages the current session's recording, the recent
tail of the debug log, and the current document's `.hew` bytes into one JSON
file. On desktop this is written to the app's log directory (in a
`reproducers` subfolder); on web it triggers a browser download. This bundle
is normally the single best attachment for a bug report, since it already
contains everything needed to reproduce the failure.

The crash screen (`ErrorBoundary.tsx`) has a **Report this crash** button for
the case the automatic dump can't cover: a kernel panic. The crash screen
appears either on an uncaught render error or, for a panic, as soon as the
panic hook fires `hew:kernel-panic` — the handler that hit the panic usually
catches the trap, so no render error follows, and the screen shows only the
underlying panic. A kernel that stopped on a trap the hook never saw (an
out-of-memory open) reaches it the same way through `recordKernelStop` (§2
above), with that stop's own heading in place of "Hew hit an error". Once a panic has been captured the automatic dump stands
down (every later uncaught error is the poisoned instance's symptom), and
the button is the way to send or save a bundle instead. It opens
`ReportBugDialog` (below) in crash mode, rendered inside the boundary itself
since the rest of the app tree is gone; the model row is unavailable there
(no live scene to save), and `recording` comes from the panic capture (§2
above) when there is one, since `take_recording()` on the registered scene
would just throw by then.

**Help ▸ Report Bug…** (`log/reportBundle.ts`, `log/reportImports.ts`,
`panels/ReportBugDialog.tsx`, docs/design/report-bug.md) opens a dialog
rather than writing a file directly: a checklist of what a report would
contain — app/system details, recorded steps, imported files, the file
that didn't open (when one didn't), the model, a log tail, and (Debug Mode
only) raw input events — each shown with its size
and a preview, any of which can be unticked before sending. Imported files
are the user files recorded calls embed as byte arrays: the `import_*`
calls, `add_texture_material`'s image, `insert_item` and
`insert_item_palette`, and a mid-session `load` (`FILE_CALLS` in
`reportImports.ts`). Unticking that row empties those arrays (and
`import_dae`'s `images`) and marks the bundle `importsStripped`, so the
steps no longer replay.
A recorded call is written only once it succeeds, so a file that fails to
open or import never reaches the recording, and a kernel that runs out of
memory aborts without running the panic hook that hands the recording over
at all. The app keeps that file itself instead (`log/failedOpen.ts`): every
`.skp` open (`runOpenSkp`, and Shop Mode's `openSkpBytes`), every import
(`runImportPick`), and every load of the user's own `.hew` (File ▸ Open,
an opened path, a drop, a recent, a Library item opened as a document, a
recovery snapshot, and Shop Mode's opens: `applyLoadedBytes` with a file,
`loadUserHewBytes`) notes its file before the kernel sees it and drops it
once the document lands, so a refusal, a panic, or an out-of-memory abort
leaves it in place. Loads of Hew's own bytes (New's blank document, a
bundled sample, an API load) note nothing and forget a failure that has
settled, so a report about something else never carries it. The report lists it as **File that
didn't open**, on by default, in both Help ▸ Report Bug… and the crash
screen's report (the kernel-error banner offers **Report Bug…** beside
**Reload**, since a reload forgets the file), and writes it under
`failedFile`:
`{"name", "label", "size", "error", "bytes"}`, where `bytes` is the file in
base64 and `error` is null when the kernel stopped before it could report
one. Extract it under its own name with
`jq -r .failedFile.bytes report.json | base64 -d > "$(jq -r .failedFile.name report.json)"`. The log line
for a failed open gives the file's size and the kernel's memory
(`kernelMemoryBytes()` in `wasm/kernelMemory.ts`): wasm memory only grows,
so a figure near 4 GB marks an out-of-memory abort, and the line says
`; out of memory` (or `; kernel trap`, or `; kernel panic` when the hook
had already captured one) when a wasm trap ended the open. The log line
for an open that worked (`Opened SKP: …`) ends with the kernel's memory
too, so a report about a model that opened but left little room says how
much it took.
A sent report that had to leave the file out still carries `failedFile`
without `bytes` — `{"name", "label", "size", "error", "omitted": "too large
to send"}` — and a top-level `dropped` array naming every ticked row Send
left out (`"model"`, `"imports"`, `"failedFile"`, in drop order), right
after the fixed-order head keys. Neither appears in a saved file or in a
report that fit.
**Send report** gzips the bundle and submits it to a private intake service
on Hew's Cloudflare account, leaving out the model, then imported files,
then the file that didn't open when it passes 1 GiB compressed, and returns a short report ID; **Save to
file…** writes every ticked row as uncompressed JSON locally instead, for
attaching to a manually-filed issue. A report after a large import runs to
hundreds of megabytes, so the bundle is never held as one string: it is
written in pieces of at most 1 MiB that are compressed or encoded as they
come, with a task boundary after each piece (browsers compress inside
`CompressionStream` writes that resolve as microtasks, so without one a
whole report compresses as a single long task). The dialog paints first and gathers once per opening (one
`peek_recording`, one `save`, one scan of the recording), showing
"Gathering details…" until that's done; ticking rows recomputes the
checklist from what was gathered, and only the compressed size is measured
again, cancellably, with one build running at a time. Unlike the automatic dump, this is user-triggered
rather than failure-triggered, so it additionally carries OS/GPU/app-version,
and it **peeks** the session recording (`peek_recording()`) rather than
taking it — a menu action has no reason to steal the calls a later crash
reproducer might still need.

If a bug doesn't trigger an unhandled error (a wrong result rather than a
crash), Help ▸ Report Bug… still works — or attach these three things by
hand to a manually-filed issue instead:

1. The **debug log** — enable Debug Mode first if it wasn't already on, so
   the file exists; download it via Settings → Debug on web, or locate
   `diagnostic.log` in the app log directory on desktop.
2. The **session recording** — obtained via `take_recording()` (§2 above), or
   from an existing report/reproducer bundle's `recording` field.
3. The **`.hew` file** you were working in when the problem occurred.

A report sent through the dialog never becomes public by itself. To also
file a public issue, use the dialog's "Also open a public GitHub issue" link
after sending (prefilled with the description, version, platform, and report
ID, never an attachment), or file one directly at
https://github.com/hew3d/hew/issues with the same three files attached by
hand.

### Attachments, follow-up uploads, and followed reports

A bundle is format 1. It may carry one optional top-level `attachment` key,
`{"name", "label", "size", "bytes"}`, with `bytes` in base64 as `failedFile`
uses it. Follow-up bundles use this key. A file sent as a reply to a report
(**Attach a file…** in Help ▸ Sent Reports, or the one-click attach for a
file that failed to open) is uploaded as a new report whose
`report.description` starts with `File for report HEW-XXXX-XXXX: <file name>`,
optionally followed by a blank line and the reporter's own note. The head
scanner reads only `format`, `report`, and `system`, so these bundles pass
through it unchanged.

A follow-up upload is the normal chunked upload, except that START carries
two extra headers: `Hew-Follow-Up: <parent ID>` and
`Hew-Follow-Secret: <key>`. START with `Hew-Follow: 1` returns an extra
`follow` field in its 201 body.

Thread routes take the same `Hew-Follow-Secret: <key>` header. Every failure
returns 403 with `{"error":"forbidden"}`.

- `GET /report/<id>/thread?after=<seq>` reads messages after a sequence number.
- `POST /report/<id>/thread` with `{"text"}` adds a reply.
- `POST /report/<id>/thread/seen` with `{"upTo"}` marks messages as seen by the reporter.
- `DELETE /report/<id>/thread` deletes the conversation.

The desktop app keeps followed reports in `followed-reports.json` in the app's
config folder, readable only by the user. Each entry holds the report ID, the
follow key, the date sent, the Hew version, the first line of the description,
and the time of the last check. The web build keeps the same records in the
app's local storage. In debug builds, `HEW_REPORT_URL` sets the base URL for
the thread routes as well as the upload routes.
