# Using the Hew API

The Hew API is how a program builds and edits a Hew model: draw a
profile, push/pull a face, group the result, save the file. Commands sit
at the altitude of the tools a person uses, so there is no vertex or
face assembly surface to look for. That is also why no sequence of them
produces a broken solid — the worst answer you can get is a refusal.

Three documents cover the API, and they are not interchangeable:

| Document | Read it for |
| --- | --- |
| This guide | How to connect, what a session looks like, working examples |
| [API_REFERENCE.gen.md](API_REFERENCE.gen.md) | Every command's parameters, results, and refusals |
| [HEW_API.md](agents/HEW_API.md) | The normative protocol rules |

Start here, keep the reference open in another tab, and go to the spec
when you need the exact rule rather than the working example.

## Install `hew-cli`

`hew-cli` is the client, and it ships with every install of the desktop
app — installing Hew installs the CLI too. Only the Homebrew cask and the
Linux `.deb`/`.rpm` add it to your shell's `PATH` automatically, so
scripts and MCP configuration below reference the full path:

| Platform | Where it lands |
| --- | --- |
| macOS | Inside the app bundle: `/Applications/Hew.app/Contents/MacOS/hew-cli` |
| macOS, Homebrew (`brew install --cask hew3d/tap/hew`) | In the same app bundle, and linked onto `PATH` as `hew-cli` |
| Windows | Next to `hew.exe` in the install directory — by default a per-user install at `%LOCALAPPDATA%\Hew\hew-cli.exe` |
| Linux, `.deb`/`.rpm` install | On `PATH` as `hew-cli` |
| Linux, AppImage | Bundled inside the image but not runnable without extracting it first (`./Hew.AppImage --appimage-extract`, then `squashfs-root/usr/bin/hew-cli`) — install the `.deb`/`.rpm` instead if you want `hew-cli` for scripting or MCP |

You can also build it from source. You need the Rust toolchain and
nothing else — the CLI needs none of the app's Node or Tauri
prerequisites (see [DEVELOPMENT.md](dev/DEVELOPMENT.md) for the full set).

```sh
git clone https://github.com/hew3d/hew
cd hew
cargo build --release -p hew-cli
```

The binary lands at `target/release/hew-cli`. Everything below uses it.

You do not need the desktop app. `hew-cli` embeds the Hew kernel, so it
creates, edits, renders, and saves documents with no UI process running
anywhere — on a laptop, on a build machine, in CI. Driving the app you
have open on screen is a separate mode, covered under
[Headless or live](#headless-or-live).

## Build a table

Save this as `table.json`. It is a JSON array of JSON-RPC envelopes,
executed in order.

```json
[
 {"jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello",
  "params": {"protocol": 1, "client": {"name": "table-demo", "version": "1.0.0"}}},

 {"jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {}},

 {"jsonrpc": "2.0", "id": 3, "method": "hew.doc.transact",
  "params": {"label": "Table", "commands": [
   {"method": "hew.sketch.draw_rect", "as": "foot",
    "params": {"plane": {"ground": true},
               "corner_a": [0.03, 0.03, 0], "corner_b": [0.09, 0.09, 0]}},
   {"method": "hew.solid.extrude", "as": "leg",
    "params": {"region": {"$ref": "foot#/region_id"}, "distance": 0.72}},
   {"method": "hew.entity.rename",
    "params": {"id": {"$ref": "leg#/object_id"}, "name": "Leg"}},
   {"method": "hew.entity.move", "as": "pair",
    "params": {"ids": [{"$ref": "leg#/object_id"}],
               "translation": [1.08, 0, 0], "copy": {"count": 1}}},
   {"method": "hew.entity.move", "as": "four",
    "params": {"ids": [{"$ref": "leg#/object_id"}, {"$ref": "pair#/ids/0"}],
               "translation": [0, 0.68, 0], "copy": {"count": 1}}},
   {"method": "hew.group.create", "as": "legs",
    "params": {"members": [{"$ref": "leg#/object_id"}, {"$ref": "pair#/ids/0"},
                           {"$ref": "four#/ids/0"}, {"$ref": "four#/ids/1"}]}},
   {"method": "hew.entity.rename",
    "params": {"id": {"$ref": "legs#/group"}, "name": "Legs"}},
   {"method": "hew.sketch.draw_rect", "as": "slab",
    "params": {"plane": {"origin": [0, 0, 0.72], "normal": [0, 0, 1]},
               "corner_a": [0, 0, 0.72], "corner_b": [1.2, 0.8, 0.72]}},
   {"method": "hew.solid.extrude", "as": "top",
    "params": {"region": {"$ref": "slab#/region_id"}, "distance": 0.04}},
   {"method": "hew.entity.rename",
    "params": {"id": {"$ref": "top#/object_id"}, "name": "Top"}}]}},

 {"jsonrpc": "2.0", "id": 4, "method": "hew.view.snapshot",
  "params": {"view": "iso", "width": 800, "height": 600, "path": "table.png"}}
]
```

Run it:

```sh
hew-cli run table.json --out table.hew
```

`run` executes a script of envelopes in order; `--out` saves the
resulting document. Each reply prints as one JSON line on stdout, and
the transaction's reply carries one result per command, in order:

```json
{"id": 3, "jsonrpc": "2.0", "result": {"label": "Table", "results": [
  {"sketch": "skt_0", "region_id": "rgn_0_100000001", "region_ids": ["rgn_0_100000001"]},
  {"object_id": "obj_1"},
  {},
  {"ids": ["obj_2"]},
  {"ids": ["obj_3", "obj_4"]},
  {"group": "grp_5"},
  {},
  {"sketch": "skt_6", "region_id": "rgn_6_100000001", "region_ids": ["rgn_6_100000001"]},
  {"object_id": "obj_7"},
  {}]}}
```

You now have `table.hew` — a 1.2 × 0.8 m tabletop on four legs, five
watertight solids, the legs in a group — and `table.png` showing it in
isometric view.

Four mechanisms in that script carry most of the weight. The
transaction made ten commands one all-or-nothing undo entry. `$ref` fed
each command's ids to the next, so no query round-trip was needed
between them. `copy: {count: N}` on `hew.entity.move` left the original
in place and translated N duplicates instead, which is how one leg
became four. `hew.view.snapshot` let the program see the result rather
than infer it from numbers. The rest of this guide is those four, plus
what to do when a command says no.

## Every request is JSON-RPC 2.0

A request carries `id`, `method`, and `params`. A response carries the
matching `id` and either `result` or `error`.

```json
{"jsonrpc": "2.0", "id": 4, "method": "hew.query.entity", "params": {"id": "grp_5"}}

{"jsonrpc": "2.0", "id": 4, "result":
 {"id": "grp_5", "kind": "group", "name": "Legs",
  "members": [{"id": "obj_1", "kind": "object"}, {"id": "obj_2", "kind": "object"},
              {"id": "obj_3", "kind": "object"}, {"id": "obj_4", "kind": "object"}]}}
```

`params` is always a single object, never a positional array. Command
names, parameter keys, and result keys are all `snake_case`.

Ignore result fields you don't recognize; new ones appear in new
releases. Hew does the opposite with your parameters and rejects any
key it doesn't know, because a misspelled parameter is a bug worth
surfacing:

```json
{"error": {"code": -32602,
           "message": "command 0: unknown field `vector`, expected one of `ids`, `translation`, `from`, `to`, `copy`"}}
```

The message names the offending command's index in the envelope and
lists the parameters that command does take.

Units are fixed and never follow the document's display settings:
lengths are meters, angles are radians, coordinates are right-handed
with +Z up, in the world frame. NaN and infinities are invalid
anywhere in an envelope.

## Say hello, then get a document

The first request on any connection is `hew.meta.hello`. Anything sent
before it is rejected with `-32004`.

```json
{"jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello",
 "params": {"protocol": 1, "client": {"name": "table-demo", "version": "1.0.0"}}}

{"jsonrpc": "2.0", "id": 1, "result":
 {"protocol": 1, "app": {"name": "hew", "version": "0.10.0"},
  "profile": "core", "encoding": "json", "documents": []}}
```

The reply tells you which protocol version the host speaks, which
*profile* it granted you (the set of commands you may call), and which
documents it has open.

Then get a document to work on. Headless, that means `hew.doc.new` or
`hew.doc.open {path}`, and the issuing connection is attached to the
result automatically. Against a running app, the user may have several
documents open, and you pick one with `hew.doc.attach`. Commands that
touch the model answer `-32002` until a document is attached.

## Several documents at once

A running app can have many windows open, and the one in front is rarely
the one an automation wants. List them, then choose:

```
hew-cli documents --live
{"id":"doc_1","title":"Octagon box","path":null,"dirty":true,"front":true,"attached":true}
{"id":"doc_2","title":"524 E Tremaine Ave - Main House","path":"/Users/me/Hew/house.hew","dirty":false,"front":false,"attached":false}
```

`hew.meta.documents` returns the same list. `id` (`doc_N`) is unique for
the life of the app and never reused. To work in the house model while
the octagon box stays in front:

```
hew-cli dispatch hew.query.scene --live --document doc_2
hew-cli run build.jsonl --live --document "524 E Tremaine Ave - Main House"
```

`--document` takes an id, or a path, label, or title that matches exactly
one window. In a script or over the raw protocol, `hew.doc.attach
{"document": "doc_2"}` rebinds the connection, or put a `"document"` member
on any single envelope to address just that request and leave the
attachment alone:

```json
{"jsonrpc":"2.0","id":7,"method":"hew.query.scene","document":"doc_2"}
```

None of this raises a window. A background document is edited where it
sits; its viewport catches up the moment the user looks at it. Only
`hew.doc.focus {document}` moves the user's focus, so use it when they
should see something.

Create documents on the fly the same way — `hew.doc.new` opens a new
window *behind* the user's current one and attaches you to it, so scratch
work never lands in the model they are editing:

```
{"method":"hew.doc.new","params":{"label":"scratch-gears"}}
→ {"document":{"id":"doc_5","title":"Untitled","label":"scratch-gears",...}}
```

`hew.doc.open {path}` does the same for a file, or attaches to the window
that already has it. Clean up with `hew.doc.close {document,
"discard": true}`; without `discard`, a document with unsaved changes
refuses (`unsaved_changes`) so an agent cannot throw away work by
accident, and the last open document refuses (`last_document`).

## Headless or live

`hew-cli` runs headless unless you pass `--live`. That flag decides
whose document you are editing.

| | Headless (default) | Live (`--live`) |
| --- | --- | --- |
| Document | Yours, in memory | The one the user has open |
| Undo history | Yours | The user's, interleaved with their edits |
| Requires the app | No | Yes, running (`--launch` starts one) |
| Profile granted | `core` | `app` |
| Getting a document | `hew.doc.new` / `hew.doc.open` | `hew.doc.attach` (any open window), or `hew.doc.new`/`open` into a new background window |
| Saving | `hew.doc.save`, or `run --out` | The user saves |

Use headless for scripts, batch work, and CI: it runs on a machine where
Hew isn't installed. Use live when an agent should build something while
the user watches, in the user's own undo history.

In live mode, `hew-cli` finds the running instance through a per-user
discovery file, connects over a local socket, and proves itself with a
per-launch token. Nothing off the machine, and no other user on it, can
reach a document. Several instances running at once is an error listing
the candidates, and `--instance <pid>` picks one. See HEW_API.md §11.2
for the transport and its platform coverage.

Live mode serves the modeling surface — sketching, solids, transforms,
structure, materials, tags, guides, attributes, history — and it aims
the viewport too: `hew.view.camera`, `hew.view.zoom_extents`, and
`hew.view.units` all take effect in the window you are looking at.

Documents save, import, and render live too, and the same command gives the
same result as headless:

- `hew.doc.save` writes the file *through the app's own Save*, so the
  document stops being dirty and becomes bound to that file, exactly as if
  the user had pressed Save As. With no `path` it re-saves in place. To
  write a file without touching the user's document, pass `"copy": true`
  with a `path`. (A document that has never been saved, with no `path`,
  hands you its bytes instead of refusing.)
- `hew.doc.export` works as before: the app encodes the mesh and `hew-cli`
  writes the file.
- `hew.doc.import {path}` merges an STL, glTF/GLB, COLLADA, or SketchUp file
  into the document, additively and as one undo entry. `hew-cli` reads the
  file (and a COLLADA file's textures) for you and sends the bytes; a raw
  client sends `bytes_base64` itself.
- `hew.component.import_definition {bytes_base64, format}` turns a file's
  bytes (`hew`, `skp`, `glb`, `gltf`, `stl`, `dae`) into one unplaced
  component definition you place with `hew.component.place`. Sending the
  same bytes again returns the same definition (`reused: true`) instead of
  making another.
- `hew.view.snapshot` and `hew.view.line_drawing` with a `path` are written
  by `hew-cli` from the bytes the app returns, once. `hew.print.pdf` is
  rendered by `hew-cli` from a copy of the document's saved bytes, so you
  can see a background document too. With no camera named, the render
  starts from what the user is looking at (the viewport camera at the
  moment of the request).

One thing a SketchUp file carries that the others don't is its display
units. A `.skp` export opens in the app's display units live and in
meters headless, so pass `units` (`"m"`, `"cm"`, `"mm"`, `"arch"`,
`"frac_in"`, `"dec_in"`) when the file should be the same either way:

```sh
hew-cli dispatch hew.doc.export '{"format":"skp","path":"chair.skp","units":"mm"}' --file chair.hew
```

HEW_API.md §12.2 lists the few things that legitimately differ between the
two modes and why they cannot be made to agree.

## One envelope, one undo entry

`hew.doc.transact` executes commands in order, atomically. Either all
of them commit as a single labeled entry in the document's ordinary
undo history, or the first refusal aborts the whole thing and leaves
the document exactly as it was.

That undo entry is the *same* history the user's own edits occupy.
There is no API-private history: the user can undo what your program
did, one step at a time, and your program can undo itself.

Sending a mutating command as a plain request is identical to sending
it as a one-command transaction. It commits one undo entry, labeled
after the command, and the reply is wrapped the same way:

```json
{"result": {"label": "hew.attr.set", "results": [{}]}}
```

### Feed one command's result to the next

Label a command with `"as"`, then reference its result from any later
command in the same transaction. `{"$ref": "<label>#<json-pointer>"}`
is replaced by the value at that JSON Pointer before the referencing
command runs:

```json
{"method": "hew.solid.extrude", "as": "leg",
 "params": {"region": {"$ref": "foot#/region_id"}, "distance": 0.72}}
```

Pointers index into arrays too — `{"$ref": "pair#/ids/0"}` is the first
id from an array-copy's result.

Referencing an unknown label, a *later* command, or a malformed pointer
is rejected before anything runs (`-32602`). A well-formed pointer that
finds nothing in the result it names is caught at execution and aborts
the transaction with `ref_resolution_failed`.

### Rules before you plan a transaction

Transactions don't nest, and there is no way to hold one open across
envelopes. A held-open transaction would either freeze the user's
editing or interleave into it. Plan, then submit.

Each command's registry entry gives it a class, and the class governs
where it may appear:

- **Model-mutating** — sketching, solids, transforms, structure,
  materials, tags, guides, attribute writes, context. The payload of a
  transaction.
- **Read-only** — `hew.query.*`, `hew.meta.*`, `hew.attr.get`. Legal
  anywhere, including mid-transaction, where their results are
  legitimate `$ref` sources. Measure something, then use the number.
- **Solitary** — document lifecycle, `hew.history.*`,
  `hew.view.snapshot`. Legal only as the sole command of an envelope.

Entering a group or component (`hew.context.enter` / `exit`) is legal
only inside a transaction, and the transaction must balance: every
`enter` gets its own `exit`, and you may not close a frame the user
already had open. The context stack is shared document state, so a
dangling frame would silently change what the user's next stroke welds
to. `hew.query.context` reports what's currently open.

## Point at geometry three ways

**Entities have ids.** Objects, sketches, groups, components,
instances, guides, materials, and tags are addressed by opaque strings
like `obj_7` and `grp_5`. Don't parse them; the format is not a
contract. Ids are stable across undo and redo, and they survive save
and load — reopen `table.hew` and the tabletop is still `obj_7`. Sketch
sub-entities (regions, curves) are session-scoped; re-resolve those
after opening a document.

The table's four legs are a group: one container, four independent
copies. For geometry that repeats and should stay linked, make a
component definition and place instances of it instead — see
`hew.component.*` in the reference.

To add something to that group later — a fifth part built afterward, say
— use `hew.group.reparent` instead of re-grouping from scratch. It moves
listed nodes into a group (or out to the top level with `parent: null`)
without touching their geometry, as one undo entry:

```json
{"method": "hew.group.reparent", "params": {"ids": ["obj_8"], "parent": "grp_5"}}
```

**Faces and edges don't.** Under sticky geometry, a face that gets
split or merged is not "the same face" in any way the kernel could
honestly promise across edits, so there are no persistent face ids.
Commands that take a face take a *locator* — the API's version of
pointing:

```json
{"object": "obj_7", "at": [0.6, 0.4, 0.76]}
{"object": "obj_7", "ray": {"origin": [0.6, 0.4, 2.0], "dir": [0, 0, -1]}}
{"$face": "leg#top"}
```

The third form is a face token. Commands that create or reshape solids
mint them, and the reference entry for each command names its token
keys: `hew.solid.extrude` gives you `base`, `top`, and `side.<n>`;
drawing a closed shape on a face gives you `face` (the new sub-face)
and `parent`. Tokens live and die with their transaction, so you never
have to re-query to find what you just made:

```json
{"method": "hew.sketch.draw_circle", "as": "hole",
 "params": {"plane": {"face": {"$face": "slab#top"}},
            "center": [0.3, 0.2, 0.05], "radius": 0.05}},
{"method": "hew.solid.push_pull",
 "params": {"face": {"$face": "hole#face"}, "distance": -0.05}}
```

A point locator that lands on the shared edge of two candidate faces is
refused as ambiguous. Hew never guesses which one you meant.

**Points can be derived instead of typed.** Anywhere a command takes a
3D point, it also takes a symbolic locator naming a point of existing
geometry — the same magnetic points the inference engine gives a user
who is snapping with the mouse:

```json
{"point": "centroid", "of": {"object": "obj_7", "at": [0.6, 0.4, 0.76]}}
{"point": "midpoint", "of": {"edge": {"object": "obj_1", "at": [0.03, 0.03, 0.36]}}}
{"point": "bbox", "of": "grp_5", "anchor": "center"}
```

Prefer this to querying coordinates and pasting them back. A derived
point resolves against the document state the command actually runs in
— inside a transaction, that means after the preceding commands have
applied — so it is exact by construction and cannot go stale between
the query and the edit that uses it. `hew.query.resolve` resolves any
locator without mutating, which is what you want while debugging.
`hew-cli dispatch` sends a single envelope, and `--file` runs it against
a document on disk:

```sh
hew-cli dispatch hew.query.resolve \
  '{"point": {"point": "centroid", "of": {"object": "obj_7", "at": [0.6, 0.4, 0.76]}}}' \
  --file table.hew
# {"result": {"point": [0.6, 0.4, 0.7600000000000001]}}
```

## Read refusals as answers

Errors come in three classes, and they call for different handling.

| Code | Class | What it means |
| --- | --- | --- |
| `-32700`, `-32601`, `-32602` | Protocol | Malformed JSON, unknown method, bad params. Fix the request. |
| `-32001` | Protocol | The method exists but your profile doesn't grant it. |
| `-32002` | Protocol | No document attached. |
| `-32004` | Protocol | No successful `hello` yet. |
| `-32000` | **Refusal** | The request was well-formed; the kernel declined it. Document untouched. |
| `-32003` | Internal fault | A kernel invariant failed and rolled back. A bug to report. |

A refusal is the normal way Hew says no, and the class worth writing
code against. Push a face further than the object is deep:

```json
{"jsonrpc": "2.0", "id": 3,
 "error": {"code": -32000, "message": "refused",
  "data": {"refusal": "would_vanish",
           "failed_index": 2,
           "failed_method": "hew.solid.push_pull",
           "detail": {},
           "explanation": "Pushing that far would remove the whole object. Push a shorter distance, or delete the object instead."}}}
```

All five fields are always present, whether the envelope was a
transaction or a plain request. `refusal` is a stable machine name to
branch on. `explanation` is the same plain-language text the app shows
a user, which makes it directly usable as feedback to an agent or as a
message in your own UI. `failed_index` is the index of the offending
command, `0` for a plain request.

The document is untouched. Nothing is half-built, so there is nothing
to clean up before retrying — the whole point of putting a plan in one
transaction.

Treat an unrecognized `refusal` name as "refused, document untouched"
and show the explanation; new names appear in new releases. Treat a
refusal as terminal for *this attempt* rather than as a permanent fact
about the input, because a documented refusal turning into a success is
how new capability arrives.

`hew-cli run` stops at the first refusal, exits nonzero, and prints the
refusal to stderr as JSON. `hew-cli dispatch` does the same for one
envelope.

## Look at what you built

`hew.query.scene` returns the document tree with a summary per entity —
kind, name, bounding box, watertightness and health, tags, group
membership:

```json
{"document": {"objects": 5, "groups": 1, "sketches": 0, "components": 0,
              "instances": 0, "materials": 0, "guides": 0},
 "tree": [
  {"id": "obj_7", "kind": "object", "name": "Top", "watertight": true,
   "health": "solid",
   "bbox": {"min": [0.0, 0.0, 0.72], "max": [1.2, 0.8, 0.76]}, "tags": []},
  {"id": "grp_5", "kind": "group", "name": "Legs", "watertight": null,
   "bbox": {"min": [0.03, 0.03, 0.0], "max": [1.17, 0.77, 0.72]},
   "members": [ ... ]}],
 "sketches": [], "components": [], "materials": [], "guides": [], "tags": []}
```

For geometry, `hew.query.faces` returns each face's plane, area,
centroid, and boundary loops:

```json
{"faces": [
  {"object": "obj_7", "area": 0.96, "centroid": [0.6, 0.4, 0.72],
   "plane": {"normal": [0.0, 0.0, -1.0], "point": [0.0, 0.0, 0.72]},
   "outer": [[0.0, 0.0, 0.72], [0.0, 0.8, 0.72], [1.2, 0.8, 0.72], [1.2, 0.0, 0.72]],
   "holes": [], "material": null, "surface": null, "texture_pins": null}, ... ]}
```

`hew.query.measure` gives distance and delta between any two points,
edges, or faces. `hew.query.raycast` is the programmatic click.

For anything a query can't tell you — proportion, alignment, whether it
looks like a table — render it. `hew.view.snapshot` takes a named view
(`iso`, `front`, `top`, and so on) or an explicit camera. It returns
PNG bytes base64-encoded, or writes the file when you pass `path`:

```sh
hew-cli dispatch hew.view.snapshot \
  '{"view": "front", "width": 800, "height": 600, "path": "front.png"}' \
  --file table.hew
```

Pass `path` when you only need the file on disk. Base64 PNG at any
useful resolution runs to megabytes, which overruns an agent's
tool-result budget fast. Headless renders go through a software
rasterizer — no GPU, no viewport, no display.

`include_ids: true` additionally returns a per-pixel id buffer and the
palette it indexes, so you can answer "which object is at pixel
(x, y)" without guessing from color.

`include_depth: true` and `include_normals: true` add the two passes an
image model wants as conditioning: a grayscale depth image (near is white,
background black; `depth_bits: 16` for finer steps) and a camera-space
normals image. The depth result carries `depth_near` and `depth_far` in
meters so a pixel value converts back to a distance (HEW_API.md §7 has the
formula). With `path`, each lands beside the snapshot as
`<path>.depth.png` and `<path>.normals.png`. They come from the same render
and camera as the color image, so the three line up pixel for pixel.

## Repair an imported model

Everything you build through the API is a solid. An imported model
often is not: SketchUp lets people draw single faces, leave a box
without its lid, or split one shape into pieces that only meet edge to
edge. `hew.doc.health` lists every object that is not a solid, and what
kind of open it is:

```json
{"rows": [
  {"object": "obj_41", "name": "Tank", "definition": "cmp_12",
   "definition_name": "Tank", "placements": 2, "instances": ["ins_13", "ins_19"],
   "accepted": false, "family": null,
   "verdict": {"kind": "cappable"},
   "rims": [{"role": "outer", "planar": true, "cappable": true, "closed": true,
             "sheet_edge": false, "shared": false,
             "perimeter": 1.62, "extent": 0.58, "edges": 32}],
   "branching_vertices": 0, "islands": 1, "faces": 5}],
 "families": [],
 "counts": {"objects": 214, "solids": 198, "flat_sheets": 12, "bent_sheets": 1,
            "cappable": 2, "holes": 1, "mixed": 0, "non_manifold": 0,
            "seam_families": 0}}
```

`flat_sheet` and `bent_sheet` are surfaces: drawn that way on purpose,
not broken. `hew.solid.thicken` pulls a flat one into a solid, and
`hew.solid.mark_surface` records that it is meant to stay a surface.
`cappable` (one flat opening around the object) and `holes` with
`planar` openings close with `hew.solid.cap`. `mixed` and
`non_manifold` need a person. Each rim says whether a face can close
it (`cappable`), and `shared` marks a seam between pieces of one shape
the importer split (rows with the same `family`): capping only the
shared rims rejoins the shape without closing openings it was drawn
with. A component's member is one row, however
many instances place it, and repairing it repairs every instance.

To close every opening a face can close in a file, cap each object on
its own, so one refusal skips that object instead of stopping the rest.
`dispatch --file` saves the file back after each repair:

```sh
cp house.hew fixed.hew
hew-cli dispatch hew.doc.health --file fixed.hew \
  | jq -r '.result.rows[] | select(any(.rims[]; .cappable)) | .object' \
  | while read -r id; do
      hew-cli dispatch hew.solid.cap "{\"object\": \"$id\"}" --file fixed.hew \
        || echo "left $id open" >&2
    done
```

A cap that cannot be made answers with a refusal that says why
(`rim_not_planar`, `caps_overlap`, …), and the object stays as it was.
Inside one transaction, `hew.solid.cap` names its new faces `cap.1`,
`cap.2`, …, so a following `hew.solid.push_pull` can work on the lid it
just made: `{"face": {"$face": "lid#cap.1"}, "distance": 0.02}`.

## Undo carefully in a live session

`hew.history.undo` and `redo` act on the top of the shared history —
whoever authored it. In a `--live` session that is a blunt instrument,
so check before you pop:

```json
{"method": "hew.history.status"}
{"result": {"undo_depth": 1, "redo_depth": 0,
            "top": {"label": "Recess", "origin": {"connection": "hew-cli:run"}}}}
```

`origin` is `user` for edits made in the app and the connection's
identity for edits made through the API. Passing `expected_label` to
`undo` makes the check atomic — it refuses rather than popping an entry
that isn't the one you meant:

```json
{"method": "hew.history.undo", "params": {"expected_label": "Recess"}}
```

History is session state. It is not written to `.hew`, so a freshly
opened document has `undo_depth: 0`.

## Store your own data on entities

Any entity, and the document itself, can carry attribute dictionaries:
namespaced JSON that Hew stores, round-trips through `.hew`, and never
interprets.

```sh
hew-cli dispatch hew.attr.set \
  '{"target": "obj_7", "ns": "com.example.bom",
    "key": "sku", "value": {"part": "TOP-1200", "qty": 1}}' \
  --file table.hew

hew-cli dispatch hew.attr.get '{"target": "obj_7"}' --file table.hew
# {"result": {"com.example.bom": {"sku": {"part": "TOP-1200", "qty": 1}}}}
```

Namespaces are reverse-DNS strings owned by whoever coined them. `hew`
and `hew.*` belong to Hew: writing one refuses
`reserved_attr_namespace`. The match is exact, so `hewlett.example` is
your namespace and goes through. Reading a `hew.*` dictionary is
allowed. Writes are ordinary mutations — transactional and undoable. The contract that matters: your data survives a
round-trip through clients, and users, that know nothing about it.

## Paint with an image texture

A texture material is an image plus the real-world size one tile of it
covers, just like the palette's Add Texture. Give `hew-cli` a file path
and it reads the image for you, headless or live. A PNG or a JPEG works.

```sh
hew-cli dispatch hew.material.create \
  '{"name": "White Oak", "texture": {"path": "oak.jpg", "world_size": [0.2, 1.0]}}' \
  --file table.hew
# {"result": {"label": "hew.material.create", "results": [{"material": "mat_9"}]}}

hew-cli dispatch hew.material.set_default \
  '{"id": "obj_7", "material": "mat_9"}' --file table.hew
```

On a face that points up, the image's height runs along x, so an image
with up-and-down grain follows the tabletop's 1.2 m length. To turn it,
scale it, or slide it on one face, do what the Position Texture tool
does: pin the corners of one tile. `origin` is a corner, `u` is one tile
along the image's width, and `v` is one tile along its height. This
turns the grain a quarter turn, across the top:

```sh
hew-cli dispatch hew.material.position_texture \
  '{"face": {"object": "obj_7", "at": [0.6, 0.4, 0.76]},
    "pins": {"origin": [0, 0, 0.76], "u": [0.2, 0, 0.76], "v": [0, 1.0, 0.76]}}' \
  --file table.hew
```

`hew.query.faces` reads each face's pins back, and
`{"face": …, "reset": true}` puts a face back to the default layout.
Positioning is one undo entry, like the tool's.

Two more palette moves work the same way. `hew.material.replace` is the
Paint tool's Shift-click: everything painted one material becomes
another, across the model or, with `"object"`, inside one object. And
`hew.library.save` with a `material` saves it to your Library, where
`hew.library.insert` adds it back to any model's palette:

```sh
hew-cli dispatch hew.material.replace \
  '{"from": "mat_3", "to": "mat_9"}' --file table.hew

hew-cli dispatch hew.library.save \
  '{"material": "mat_9", "name": "White Oak"}' --file table.hew
# {"result": {"path": "Materials/white-oak-5b1c07.hew", "id": "5b1c07…"}}
```

## Save and insert library items

`hew.library.*` is the same Library a person browses with Window ▸
Library — a folder of `.hew` files a script or an agent can read and
write too, on any host with a filesystem. Save the table you built
above as a reusable component:

```sh
hew-cli dispatch hew.library.save \
  '{"selection": ["obj_1"], "name": "Side Table", "keywords": ["furniture", "table"]}' \
  --file table.hew
# {"result": {"path": "Components/side-table-3f2a99.hew", "id": "3f2a9900-…"}}
```

Leave out `selection` and it saves the whole document as a `model`
item instead. `hew.library.list` browses what is there — filter by
`category`, `collection`, or a text `query` — and `hew.library.describe`
gets one item's full metadata and manifest counts:

```sh
hew-cli dispatch hew.library.list '{"category": "component"}' --file table.hew
# {"result": {"folder": "/Users/you/Hew Library", "items": [{"path": "Components/side-table-3f2a99.hew", "name": "Side Table", …}]}}
```

Insert it into a document — by the id `save` returned, or by its
path — and it lands wired for reuse: inserting the same item version
again shares the existing definition instead of copying it a second
time, exactly like dragging it from the Library browser.

```sh
hew-cli dispatch hew.library.insert \
  '{"item": "3f2a9900-…", "at": [2, 0, 0]}' \
  --file room.hew
```

`hew.library.remove` deletes an item; `hew.library.update_meta` edits
its name, keywords, or collection without touching the file it was
saved as. All six commands work identically in `--live` — the
Library lives on whichever machine runs `hew-cli`, so `list`/
`describe`/`remove`/`update_meta` never even need a running desktop
instance, and `insert`/`save` read or write the item on this side
while the geometry itself lands in the live document (§12.1 of the
spec has the full mechanics).

Two differences from saving through the app's own Library browser,
worth knowing: `save` stamps the item's "saved from" bookkeeping
(`sourceDoc`) from the document `--file` opened automatically — pass
`source_doc` yourself to override it, or when dispatching against a
fresh, never-saved document there is nothing to default from and the
field is just left off. And an item saved through the API gets no
thumbnail — the app renders and caches one at save time, but there is
no headless equivalent of that render yet, so the item shows a
generic tile in the Library browser until a UI session opens the
folder and saves over it.

## Connect an AI agent over MCP

`hew-cli mcp` serves the Model Context Protocol on stdio. It is a
standard stdio server block, so it goes wherever your MCP client keeps
those — a project's `.mcp.json`, Claude Desktop's
`claude_desktop_config.json`, or the equivalent for your host. `command`
is the [`hew-cli` path for your platform](#install-hew-cli):

macOS:

```json
{
  "mcpServers": {
    "hew": {
      "type": "stdio",
      "command": "/Applications/Hew.app/Contents/MacOS/hew-cli",
      "args": ["mcp"]
    }
  }
}
```

A Homebrew install lives at that same path. Keep the full path there
even though `hew-cli` is on your `PATH`: an MCP host launched from the
Dock or Finder does not inherit your shell's `PATH`, so a bare
`hew-cli` may not be found.

Windows (substitute your username for `<you>`):

```json
{
  "mcpServers": {
    "hew": {
      "type": "stdio",
      "command": "C:\\Users\\<you>\\AppData\\Local\\Hew\\hew-cli.exe",
      "args": ["mcp"]
    }
  }
}
```

Linux, installed from a `.deb`/`.rpm` (`hew-cli` is already on `PATH`):

```json
{
  "mcpServers": {
    "hew": {
      "type": "stdio",
      "command": "hew-cli",
      "args": ["mcp"]
    }
  }
}
```

Add `"--live"` to `args` to drive the app the user has open instead of
an embedded document.

The server does not expose one tool per command. Eighty near-flat tools
would bury a model in choices while stripping the transaction semantics
that make multi-step work atomic. It serves seven, and the list is
computed at connection time from the profile you were granted, so an
agent never sees a tool it cannot call:

| Tool | What it does |
| --- | --- |
| `hew_transact` | The workhorse. A full transaction; every model edit goes through it. |
| `hew_query` | Any read-only command and its params. |
| `hew_describe_scene` | The document tree as a summary tuned for reasoning over. |
| `hew_snapshot` | Render to PNG, so the agent sees what it built. |
| `hew_print_pdf` | Print the document to a PDF at an exact scale, tiled and titled like File ▸ Print…. |
| `hew_line_drawing` | A hidden-line SVG, or raw 2D segments, for a camera, a standard view, or a Scene. |
| `hew_capabilities` | Every command's schema and refusal inventory, at run time. |

The handshake and the working document are handled for you — a
`hew_transact` call works as the first thing an agent does.

The loop that works: describe the scene, plan, `hew_transact`, read the
result or the refusal, snapshot, continue. Refusal explanations are
what the agent self-corrects on, and the transaction guarantee means a
failed plan leaves nothing half-built behind.

## Call it from TypeScript

`app/src/api/hewApi.gen.ts` is a typed client generated from the same
registry, with one method and one `Params`/`Result` pair per command,
grouped by namespace. It is a single dependency-free file rather than a
published package: Hew's own frontend imports it in place, and an
outside project vendors a copy. It carries no transport of its own — you
supply anything that can move one envelope and bring back a response:

```ts
import { HewApiClient, HewApiError, type HewTransport } from './api/hewApi.gen'

const transport: HewTransport = {
  async dispatch(request) {
    const response = await fetch('/hew', {
      method: 'POST',
      body: JSON.stringify(request),
    })
    return response.json()
  },
}

const hew = new HewApiClient(transport)

await hew.meta.hello({ protocol: 1, client: { name: 'my-app', version: '1.0.0' } })
await hew.doc.new({})

const { results } = await hew.doc.transact({
  label: 'Table',
  commands: [
    { method: 'hew.sketch.draw_rect', as: 'foot', params: { /* ... */ } },
    { method: 'hew.solid.extrude', params: { region: { $ref: 'foot#/region_id' }, distance: 0.72 } },
  ],
})
```

Single mutating commands are unwrapped for you: `hew.solid.extrude(…)`
resolves with `{object_id}` rather than the one-command transaction
envelope around it. Any error response throws `HewApiError`, whose
`refusal` field carries the `-32000` payload described above.

Do not edit the generated file. Regenerate it with:

```sh
REGENERATE_API_ARTIFACTS=1 cargo test -p api --test generate_artifacts
```

## Ask what this build supports

`hew.meta.capabilities` returns the registry as data — for every
command your connection was granted, its name, class, whether it is
implemented, its parameter and result schemas, and its refusal
inventory:

```json
{"commands": [
  {"name": "hew.attr.delete", "class": "model_mutating", "implemented": true,
   "summary": "Delete one attribute key or a whole namespace.",
   "params": { ... }, "result": { ... },
   "refusals": ["unknown_entity", "invalid_attr_name", "unknown_attr", ...]}]}
```

Ask, rather than assuming, whenever a client has to work against more
than one version of Hew. The reply carries the same declarations the
committed TypeScript client and API reference are generated from, so it
is enough to generate a client SDK against a running host.

The protocol evolves additively and stays at version 1 as long as
possible. New commands, new optional parameters, new result fields, and
new refusal names appear in any release. What never changes: the
meaning of an existing command given existing parameters, and the type
or meaning of an existing field. A change that can't be made additively
ships as a new command beside the original.

## `hew-cli` reference

```
hew-cli run <script.json|script.jsonl> [--out <file.hew>]
             [--live [--launch] [--instance <pid>] [--document <ref>]]
hew-cli dispatch <method> [params-json] (--file <model.hew> | --live [...])
hew-cli documents --live [--launch] [--instance <pid>]
hew-cli mcp [--live [--launch] [--instance <pid>] [--document <ref>]]
```

`run` executes a script of raw envelopes in order. A `.json` file is
one JSON array, so you can indent it; a `.jsonl` file is one envelope
per line. The script speaks for itself — it opens with its own
`hew.meta.hello` and its own `hew.doc.new`, `open`, or `attach`. Each
reply prints as one line on stdout; the first refusal stops execution
with exit 1. `--out` saves the resulting document, headless only.

`dispatch` sends one envelope and prints the reply. `--file` opens a
`.hew` document, applies the command, and saves it back in place unless
the command is read-only. `--live` sends it to a running app instead.
One of the two is required; `hew-cli` never guesses.

A script is a reproducible artifact, which makes one useful as a test.
Log the envelopes your client sends, replay them with `hew-cli run
--out`, and compare the result against a committed `.hew`. The kernel is
deterministic, so the bytes match or something regressed.
