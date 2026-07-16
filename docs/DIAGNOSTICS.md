# Hew — Diagnostics

Hew ships three related diagnostic facilities: a structured debug log, a
session recording/replay mechanism, and an automatic crash-reproducer bundle
that combines both. Together they let a bug report include enough state to
reproduce the problem deterministically, rather than a description of what
was on screen.

This document specifies the on-disk formats normatively (so a bug report can
be inspected or replayed by anyone, without proprietary tooling) and
describes how to enable, read, and attach them.

## 1. Debug log

### What it captures

The kernel (compiled to WASM) and the UI each emit structured events into one
merged, time-ordered stream:

- **Kernel events** — one record per `tracing` event raised by the kernel and
  inference crates: the start of each document-mutating operation (name +
  scalar parameters), the resulting canonical state digest after it commits,
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
| `kernel::cmd` | INFO | after each committed mutation | `state_hash` (canonical digest after the op), `objects` (count touched) |
| `inference::resolve` | TRACE | every snap query (per pointer move) | `candidates` (count), `winner` (snap kind, or `None`) |

A single user gesture therefore brackets as one `kernel::op` record (the
command and its parameters) followed by one `kernel::cmd` record (its
effect on the canonical state), sharing a `corr` value.

Well-known `fields` keys:

- `op` — command name, matching the underlying document method.
- `state_hash` — the document's canonical state digest after the operation.
  Two runs that are supposed to be identical (e.g. original vs. replay) can be
  compared op-by-op by walking matching `state_hash` values, which pinpoints
  the exact operation at which they diverge.
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

Only mutations that actually succeeded are recorded — a rejected or failed
operation never appears, so a recording can't reproduce an operation that
never really happened.

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
    { "method": "delete_node", "kind": 0, "id": 4 }
  ],
  "golden_hash": 5678,       // u64. Document state digest after the last
                             //   call — the replay oracle.
  "input": [ /* optional, see below */ ]
}
```

Each entry in `calls` is a tagged object: `method` selects the call, and the
remaining fields are its arguments. `sketch`/`region`/`object`/`id` are the
literal internal handles from the recording session.

| `method` | Arguments | Effect |
|---|---|---|
| `begin_ground_sketch` | — | begin a ground-plane sketch |
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
| `group_nodes` | `kinds[]`/`ids[]` (parallel node lists, kind as `delete_node`) | form a merge group |
| `duplicate_node` | `kind`, `id`, `affine[12]` (row-major 3×4) | deep-copy a node (Move+Alt) at the affine offset |
| `slice_object` | `object`, `plane[6]` (`[px,py,pz,nx,ny,nz]`) | slice an Object by a plane |
| `split_face_inner` | `object`, `face`, `loop_pts[]` (xyz triples), optional `curve[4]` (`[cx,cy,cz,radius]`) | imprint a closed loop on a solid face (draw-on-face); `curve` present ⇒ the loop is a circle carrying its analytic identity, so a later push-through stamps the tunnel walls |
| `push_pull` | `object`, `face`, `distance` | push/pull a solid face; the kernel re-derives the routing (translate, whole-wall radial offset, boss/recess, or through-cut) on replay |
| `transform_object` | `object`, `affine[12]` (row-major 3×4) | apply an affine transform |
| `transform_selection` | `kinds[]`/`ids[]` (parallel node lists), `sketches[]`, `affine[12]` | transform a whole multi-selection as one undo step |
| `delete_node` | `kind` (0=object, 1=group, 2=instance), `id` | delete a node |
| `duplicate_node` | `kind`, `id`, `affine[12]` | Move+copy of one node: a deep clone placed at the offset |
| `duplicate_selection_array` | `kinds[]`/`ids[]` (parallel node lists), `affine[12]`, `count` (1–1000; an out-of-range count fails the replay typed rather than hanging) | array copy: every listed node cloned `count` times along the step, one undo step |
| `scene_undo` / `scene_redo` | — | document-level undo/redo (recorded only when it succeeded) |
| `transform_sketch` | `sketch`, `affine[12]` | bake an affine into a free-standing sketch |
| `transform_sketch_island` | `sketch`, `island`, `affine[12]` | rigidly move one sketch island |
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
| `push_pull_in_component` | `component`, `object`, `face`, `distance` | push/pull a face inside a component definition |
| `split_face` | `object`, `face`, `path[]` (xyz triples) | cut a face along a drawn path |
| `merge_faces` | `object`, `edge` | dissolve the boundary between two coplanar faces |
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
| `add_guide_line` | `origin[3]`, `dir[3]` | add a construction line |
| `add_guide_point` | `p[3]` | add a construction point |
| `delete_guide` | `guide` | delete one construction guide |
| `delete_all_guides` | — | delete every visible guide in one undo step |
| `import_dae` | `bytes[]`, `images[]` (`{uri, bytes[], format}` objects) | additive COLLADA import (embeds the file + images) |
| `import_gltf` | `bytes[]` | additive glTF/GLB import (embeds the file) |
| `import_skp` | `bytes[]` | additive `.skp` import (embeds the file) |
| `load` | `bytes[]` | replace the whole document — a mid-session File ▸ Open/New (embeds the `.hew` bytes) |

**Coverage rule** (ratified with the recording audit): every `Scene` method
that pushes the document undo stack or mutates state included in
`Document::save` records itself. Anything less makes `scene_undo` /
`scene_redo` — which ARE recorded — replay against a differently-shaped
stack and silently reproduce a different session. Session-only state that
is neither undoable nor saved (inference hide sets, transient snap
segments, guide/axis snappability, torture mode) is deliberately not
recorded. Calls that carry file or image payloads (`import_*`, `load`,
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

Discriminated by `kind`: `pointerdown` / `pointermove` / `pointerup` /
`camera` / `keydown`. `seq` is a global monotonic counter; `t` is milliseconds
since capture start (pacing, not identity); `gesture` is a per-gesture
correlation id that increments on each `pointerdown`. Pointer coordinates are
CSS pixels relative to the viewport canvas's top-left corner. A `camera`
event carries `position` / `target` / `up` / `fovDeg`, enough to rebuild the
camera and its orbit target. A recording with only `calls` (no `input`) is
still a complete, valid recording — the input layer is a diagnostic
supplement, not a requirement.

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
Hew capture it automatically: see. To capture one manually while
scripting or driving the app programmatically, the underlying calls are
`start_recording()`, `stop_recording()`, `is_recording()`, and
`take_recording()` (returns the recording JSON above and clears the buffer).

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

If a bug doesn't trigger an unhandled error (a wrong result rather than a
crash), attach these three things by hand instead:

1. The **debug log** — enable Debug Mode first if it wasn't already on, so
   the file exists; download it via Settings → Debug on web, or locate
   `diagnostic.log` in the app log directory on desktop.
2. The **session recording** — obtained via `take_recording()` (see), or
   from an existing reproducer bundle's `recording` field.
3. The **`.hew` file** you were working in when the problem occurred.

File bugs at https://github.com/hew3d/hew/issues. Include the Hew version,
platform, and a short description of what you expected versus what happened
— the attached files carry the reproducible detail.
