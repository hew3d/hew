# Hew Native File Format (`.hew`)

This document specifies the current `.hew` container format — zip layout,
JSON manifest schema, binary geometry buffer layout, and semantics — in
enough detail for an independent implementation to produce byte-compatible
output and correctly interpret every field, with no access to Hew's source.

Two independent format numbers appear in every file: **manifest format
version `8`**, and **geometry buffer format version `3`**. Both are covered
below, including exactly which fields exist at each version and how a
reader must treat versions it does not recognize.

## 1. Container

A `.hew` file is a **zip archive**. Every entry is stored with the **`stored`
(uncompressed) method** — geometry buffers are already compact binary, and
texture bytes are already compressed in their source image format, so
DEFLATE would add CPU cost for negligible size savings.

| Entry path | Contents |
|---|---|
| `manifest.json` | UTF-8 JSON. Document tree, materials, sketches, guides, metadata. Exactly one per file. |
| `geometry/obj_<id>.bin` | One binary geometry buffer per live object. `<id>` is the object's dense id (see), decimal, no leading zeros. |
| `textures/tex_<id>.<ext>` | Verbatim authored image bytes for one textured material. `<id>` is the material's dense id. `<ext>` is `png` or `jpg`, matching the material's `texture.format`. |

A file with zero textured materials has no `textures/` entries at all; a file
with zero objects has no `geometry/` entries. `manifest.json` is always
present — its absence, or an unreadable zip container, is a fatal read error.

### Determinism

Writers MUST produce byte-identical output for byte-identical input — saving
the same in-memory document twice yields the same file, to the last byte.
This requires:

- **Fixed entry order**: `manifest.json` first, then `geometry/obj_*.bin` in
  ascending dense-id order, then `textures/tex_*` in ascending dense-id order
  — never alphabetical, never unordered-map iteration order.
- **Fixed zip metadata**: every entry's last-modified timestamp and Unix
  permission bits are written as fixed constants on every save, never
  derived from the wall clock or filesystem. The specific constants carry no
  meaning; only their invariance across saves matters.
- **Deterministic dense ids and array ordering** (): every array in the
  manifest is emitted in dense-id order, never re-sorted or hash-ordered.

A reader, by contrast, MUST NOT depend on any of the above: it must accept
entries in any physical order and tolerate any timestamp/permission values.

## 2. Manifest (`manifest.json`)

The manifest is a single JSON object. All arrays below are emitted in
ascending dense-id order (the array index equals the entry's own `id` field).

```jsonc
{
  "format_version": 9,
  "geometry_version": 3,
  "app": "hew",
  "app_version": "0.1.0",

  "materials": [
    { "id": 0, "name": "Red", "color": [220, 50, 40, 255],
      "texture": {                  // omitted entirely for a flat-color material
        "asset": "textures/tex_0.png",
        "format": "png",            // "png" | "jpg"
        "world_size": [1.0, 1.0]    // meters per image tile [width, height], both > 0
      } }
  ],
  "objects": [
    { "id": 0, "geometry": "geometry/obj_0.bin", "base_material": 0,
      "name": "Counter_Base",
      "tags": [["Architecture", "Walls"], ["Level", "L1"]],
      "footprints": [               // v9+; omitted when the object has none
        { "sketch": 0,              // dense sketch id the footprint shadows
          "outer": [[0,0,0], [4,0,0], [4,4,0], [0,4,0]],
          "holes": [ [[1,1,0], [1,2,0], [2,2,0], [2,1,0]] ] } ] }
  ],
  "groups": [
    { "id": 0, "members": [ {"kind":"object","id":0}, {"kind":"group","id":1} ],
      "name": "MyGroup",
      "tags": [["Architecture"]] }
  ],
  "components": [
    { "id": 0, "members": [2, 3], "name": "MyComponent" }
  ],
  "instances": [
    { "id": 0, "def": 0,
      "pose": [1,0,0,0, 0,1,0,0, 0,0,1,0],
      "name": "Chair_1",
      "tags": [["Furniture"]] }
  ],
  "sketches": [
    { "id": 0,
      "plane": [0.0, 0.0, 1.0, 0.0],
      "vertices": [ {"id":0, "p":[0.0, 0.0, 0.0]} ],
      "edges":    [ {"id":0, "from":0, "to":1, "curve":0} ],
      "regions":  [ {"id":0, "outer":[0,1,2,3], "holes":[[4,5,6]]} ]
    }
  ],
  "guides": [
    { "id": 0, "kind": "line", "p": [0.0, 0.0, 0.0], "dir": [1.0, 0.0, 0.0] },
    { "id": 1, "kind": "point", "p": [2.0, 3.0, 0.0] }
  ],
  "roots": [ {"kind":"object","id":0}, {"kind":"instance","id":0} ],
  "consumed": [ [0, 0] ],
  "tags": [ {"path": ["Architecture", "Walls"]},
            {"path": ["Mock Walls"], "hidden": true} ]
}
```

### Field reference

- **`format_version`** (`u32`, required) — manifest schema version. Current: `9`.
- **`geometry_version`** (`u32`, required) — geometry buffer layout version
  used by every entry under `geometry/` in this file. Current: `3`.
  Redundant with the per-buffer version in each buffer's own header (),
  but lets a reader reject a whole file up front without opening buffers.
- **`app`**, **`app_version`** (`string`, required) — free-form writer
  identification, no semantic meaning. A reader MUST NOT branch on these.
- **`materials`** — the material palette ().
- **`objects`** — every live Object, both world-tree and component-definition
  members ().
- **`groups`** — merge groups: ordered, nestable membership lists ().
- **`components`** — component definitions: flat, ordered object-id sets ().
- **`instances`** — placements of a component definition at a pose ().
- **`sketches`** — first-class 2D sketches ().
- **`guides`** — construction lines and points ().
- **`roots`** — the document's top-level nodes, in display order ().
- **`consumed`** — `(sketch, region)` id pairs already extruded ().
- **`tags`** — the tag metadata registry: known tag paths with their
  hidden-by-default flags.

### `NodeRef`

A `NodeRef` — used in `roots` and `groups[].members` — is `{"kind": ..,
"id": ..}` with `kind` one of `"object"`, `"group"`, `"instance"`. There is no
`"component"` kind: a definition is never placed directly, only reached
through an `instance`.

### Optional-id convention

Anywhere a field is an *optional* reference to another entry's dense id
(e.g. `objects[].base_material`), the manifest encodes absence as JSON
`null` or by omitting the field; a reader must accept either. This is the
same "no material" value as the binary encoding's `0xFFFFFFFF` sentinel
(), just encoded per-container.

### Forward and backward compatibility

A reader that implements manifest format version *N*:

- MUST successfully load any file with `1 <= format_version <= N`. Fields
  introduced after a file's own version are absent from it; the
  field-by-version table below gives the default each absent field takes.
  A reader applies those defaults rather than treating absence as an error.
- MUST reject `format_version == 0` or `format_version > N` with a distinct,
  typed "unsupported version" error — never guessing at an unknown shape or
  falling back to a partial parse.
- MUST tolerate unrecognized JSON object keys anywhere in the manifest,
  ignoring them, so a future minor addition can coexist with older readers.
- MUST NOT silently repair structurally invalid content (a dangling id
  reference, an out-of-range index, an unknown `kind` string) — these are
  fatal, typed errors. A file either loads exactly as written or is rejected;
  there is no partial or best-effort load.

Field-by-version table — every field not listed here exists at every version
of the manifest:

| Field | First appears at `format_version` | Value when absent |
|---|---|---|
| `objects[].name`, `groups[].name`, `components[].name`, `instances[].name` | 2 | no display name |
| `objects[].tags`, `groups[].tags`, `instances[].tags` | 3 | empty list (no tags) |
| `guides` (top-level array) | 4 | empty list (no guides) |
| `tags` (top-level array) | 5 | empty registry (all node-carried tags visible) |
| `objects[].hidden`, `groups[].hidden`, `instances[].hidden` | 6 | `false` (visible) |
| `sketches[].edges[].curve` | 7 | absent (a plain line, not part of a curve chain) |
| `objects[].source` | 8 (v8 only — v9+ writers emit `footprints` instead) | absent (no extrusion provenance) |
| `objects[].footprints` | 9 | empty list (the object shadows no sketch area; nothing derives as consumed for it) |

Note `components[]` entries never carry a `tags` field at any version —
tags are attached to *placements* (objects, groups, instances), not to
definitions, since one definition may be instanced multiple times under
different tags.

## 3. Geometry buffer (`geometry/obj_<id>.bin`)

Each buffer is a standalone binary document encoding one object's geometry
as **face generators** (vertex positions plus per-face vertex-index loops)
rather than a serialized half-edge graph. A reader reconstructs full
topology from these generators and must independently verify the result is
valid — manifold, watertight-consistent, non-degenerate ().

All multi-byte integers and all floats are **little-endian**. Material ids
inside a buffer are the same dense ids used by `manifest.json`'s `materials`
array — one shared id space, not a buffer-local one.

### 3.1 Layout (current version, `3`)

```
offset  type                field
0       u8[4]               magic = "HEWG" (0x48 0x45 0x57 0x47)
4       u32                 version (currently 3)
8       u8                  watertight: 0 = Open (leaky), 1 = Watertight
9       u32                 base_material id (0xFFFFFFFF = none)
13      u8                  imported flag: 0 = strict native tolerance, 1 = imported tolerance
14      u32                 vertex_count (N)
18      f64[3] * N          vertex positions, one (x, y, z) triple per vertex, in vertex-slot order
...     u32                 face_count (F)
        --- repeated F times, in face-slot order: ---
        u32                 face material id (0xFFFFFFFF = none)
        u8                  uv_frame flag: 0 = no UV frame, 1 = UV frame follows
        f64[8]              uv_frame, present only when the flag above is 1:
                             s.x s.y s.z  t.x t.y t.z  u0 v0
        u32                 outer_count (K)
        u32[K]              outer-loop vertex indices (0-based into the vertex table, in loop order)
        u32                 hole_count (H)
        --- repeated H times: ---
        u32                 hole_vertex_count (J)
        u32[J]              hole-loop vertex indices
```

Each face's outer loop and each of its hole loops is a closed polygon: the
last index does not repeat the first (the loop is implicitly closed by
wrapping back to index 0 of that loop).

`uv_frame`, when present, is an oriented planar UV mapping: for a world
point `p` on the face's plane, `uv = (s·p + u0, t·p + v0)`, where `s`/`t`
are (not necessarily unit or orthogonal) world-space vectors and `u0`/`v0`
are scalar offsets. A face with no `uv_frame` instead uses its material's
`world_size` planar projection when textured () — mutually exclusive
paths, selected by this flag.

### 3.2 Field availability by version

A conforming writer always writes one buffer version per file, matching the
manifest's `geometry_version`; a reader should nonetheless decode each
buffer by its own header `version` field, not by trusting the manifest
value uncritically. Two fields are version-gated; every other field is
present at every version:

| Field | First appears at buffer `version` | Byte layout when absent | Effective value when absent |
|---|---|---|---|
| header `imported` flag (offset 13) | 3 | byte is not present at all — `vertex_count` begins one byte earlier | `0` (strict native tolerance) |
| per-face `uv_frame` flag + payload | 2 | flag byte and payload are not present at all — `outer_count` follows the material id directly | no UV frame |

Both gates shift subsequent byte offsets, so a reader must branch on the
buffer's own `version` at each gated point rather than assume fixed offsets
past byte 9 (`base_material`). A reader MUST reject `version == 0` or
`version` greater than the highest it implements, with a distinct
"unsupported version" error — the same rule as the manifest ().

### 3.3 Decode obligations

A decoder is not just a byte parser: it must reject any buffer whose parsed
contents do not describe valid geometry.

- **Bad magic** (first 4 bytes not `"HEWG"`) is a fatal, immediate error.
- **Truncation** (buffer ends before its own announced counts are
  satisfied) is a fatal error, distinct from corruption.
- **Structural corruption** — an out-of-range vertex index, a loop under 3
  vertices (`outer_count`/`hole_vertex_count < 3`), or a `watertight`/
  `uv_frame`/`imported` byte outside its 0-1 range — is a fatal error
  reported with the byte offset and what was wrong. Never a panic, never
  best-effort recovery.
- **Face plane recomputation.** Planes are *not* stored; a reader must
  recompute each face's best-fit plane from its outer-loop vertices and
  reject a loop that doesn't span a plane within tolerance (gated by
  `imported`). This is safe because a baked object's transform is always
  orientation-preserving (), so the recomputed plane's normal always
  matches how the geometry was authored.
- **Watertightness cross-check.** The stored `watertight` byte must equal
  what the reader's own topology reconstruction computes — a mismatch is a
  fatal error (cheap tamper detection), never silently corrected.
- **Full topology validation.** The normal topology validator (manifold
  edges, closed face loops, no orphan geometry) must pass before a decode
  counts as successful.

There is no repair path: any failed check above rejects the whole buffer
outright with a typed reason — never a partially-loaded document.

## 4. Semantics

### 4.1 Units, coordinate system, tolerances

- All lengths, in both the manifest and the geometry buffers, are **f64
  meters** — no unit field, no conversion step; convert at your own boundary.
- The coordinate system is **right-handed** with **+Z as up**. Plane
  normals, face winding, and instance pose rotations are all in this frame.
- Face winding is **counter-clockwise from the outward side** for an outer
  loop and **clockwise from the outward side** for a hole loop — applied
  consistently to geometry-buffer face loops and sketch region loops ().
- Two face-planarity tolerances exist, selected per-object by the geometry
  buffer's `imported` flag (): strict for natively-built geometry (exact
  to double precision by construction), wider for geometry recovered from a
  foreign, typically single-precision-quantized source, where a face the
  source considered flat can land measurably off its best-fit plane. Both
  are fixed constants; apply the wider one only when `imported = 1`.

### 4.2 Identity model

Every entity in a saved file — materials, objects, groups, components,
instances, sketches, guides, and, within a sketch, its own vertices/edges/
regions — has a **dense `u32` id**, unique within its own kind but not
across kinds (object `id: 0` and material `id: 0` are unrelated). Dense ids
start at 0 with no gaps: 5 live objects means ids `{0, 1, 2, 3, 4}`.

A save captures *live, visible* state only, like a flattened snapshot, not
an editing history: no undo/redo log is persisted, and any entity that
existed only transiently before the save is never written and never
referenced.

Consequently dense ids are **not stable across saves** — the same logical
object may get a different id next time the document is saved. A reader
must not cache a dense id across independent loads and expect it to mean
the same entity.

A *required* reference that does not resolve to a live entry of the
expected kind — a dangling group member, an out-of-range material id, an
unknown `NodeRef.kind` — is a fatal, typed load error; nothing is ever
silently dropped or coerced to "none."

### 4.3 Objects

An **Object** is the document's unit of solid geometry: a closed,
watertight-tracked mesh described by exactly one `geometry/obj_<id>.bin`
buffer. Its manifest entry (`objects[]`) carries `geometry` (the buffer's
zip path), an optional `base_material` (used by any face whose own per-face
material is absent — an object-level default paint, not an override), an
optional `name`, optional `tags` (), and optional `footprints` (v9+): the
sketch-plane areas the solid stands on, each a dense `sketch` id plus the
extruded profile's loops in world coordinates (`outer` counter-clockwise,
`holes` clockwise, matching sketch-region winding). Footprints are frozen
at extrusion time; boolean/slice/push-through results inherit their
operands' entries. Editors DERIVE the `consumed` region set from these
polygons — a region is consumed iff its material overlaps a live object's
footprint — so consumption survives any sketch re-topology (splits,
merges, delete-and-redraw) and lifts by itself when the last solid over
an area is deleted. A reader that ignores `footprints` loses only that
bookkeeping, never geometry; a footprint whose `sketch` is out of range
degrades to absent — never load-fatal.

v8 files carry an optional `source` instead: the dense `[sketch, region]`
pair the object was extruded from, resolved like `consumed` entries. A v9
reader converts it by freezing that region's loops as the footprint (the
file was saved with region and solid consistent). v9+ writers do not emit
`source`.

A `consumed` pair that no loaded footprint covers (pre-v9 boolean/slice/
push-through results carried no `source`; pre-v8 files attributed nothing)
is honored verbatim, never silently freed: the reader freezes that
region's loops as a document-lifetime footprint, so the area stays
consumed through later edits exactly as it did in the version that wrote
the file. Such regions remain in `consumed` on save, so the freeze
survives round trips.

Every object id appears in **exactly one** structural position: either a
member of exactly one component definition (`components[].members`), or
reachable from the world tree (directly in `roots`, or nested inside a
`groups[].members` chain) — never both at once.

Object transforms baked into a buffer's vertex positions are always
orientation-preserving (determinant positive — reflections are never baked
into geometry). A mirrored placement exists only as a component **instance**
pose, never as baked-in geometry — this is what lets the geometry decoder
recompute face-plane orientation from winding alone ().

### 4.4 Groups, components, instances

- **Groups** (`groups[]`) are a plain, ordered membership list of
  `NodeRef`s — objects or other groups, nesting unrestricted — plus an
  optional name and tags. A group has no geometry or transform of its own.
- **Component definitions** (`components[]`) are an ordered, flat list of
  member object dense ids (`members`), plus an optional name. A definition
  is never placed directly, carries no transform/tags/position, and exists
  purely to be instanced.
- **Instances** (`instances[]`) place one component definition (`def`, a
  dense component id) at a **pose**: a row-major 3×4 affine matrix
  `[m00,m01,m02,tx, m10,m11,m12,ty, m20,m21,m22,tz]`, read as `world_point =
  linear * local_point + translation` (`linear` the 3×3 block, `translation`
  the vector `(tx,ty,tz)`). Unlike baked object geometry, a pose **may** be
  reflective or non-uniformly scaled, since poses are never baked into the
  definition's geometry. Each instance carries its own optional name
  (falling back to the definition's name if absent) and optional tags.

### 4.5 Materials and textures

Each `materials[]` entry has an id, an informational `name`, and a `color`
(`[r, g, b, a]`, each `0-255`). A material may optionally carry a `texture`:
`asset` (the zip path of the verbatim image bytes), `format` (`"png"` or
`"jpg"`, matching the asset's actual encoding), and `world_size` (`[width,
height]` meters per image tile). A textured face with no per-face
`uv_frame` () is planar-projected and tiled at `world_size`; a face
that does carry a `uv_frame` ignores `world_size` and uses the frame's
affine mapping instead. A material with no `texture` renders as a flat
`color` fill.

### 4.6 Sketches

A **sketch** (`sketches[]`) is a first-class 2D construction plane with its
own sketch-local dense id spaces for vertices, edges, and regions (sketch A's
vertex id `0` is unrelated to sketch B's, or to any object/material id `0`).

- `plane` — `[nx, ny, nz, offset]`: a unit normal and offset such that a
  point `p` lies on the plane iff `n·p = offset`.
- `vertices[]` — `{id, p}`, a 3D point. A reader need not verify `p` lies on
  `plane`, though a well-formed writer only ever places it there.
- `edges[]` — `{id, from, to, curve?}`: an undirected connection between
  two sketch-local vertex ids. `curve` (v7+, optional) is a sketch-local
  dense id naming the **curve chain** the edge belongs to: the facets a
  single drawn arc or circle committed together, which editors treat as one
  selectable unit. Edges sharing a `curve` value belong to the same chain;
  an absent `curve` is a plain line. Curve ids are dense per sketch in
  first-appearance order over the edge list and carry no geometry of their
  own — a reader that ignores them loses only selection grouping, never
  shape.
- `regions[]` — `{id, outer, holes}`: a closed planar polygon-with-holes.
  `outer` is a vertex-id cycle, counter-clockwise from the plane normal's
  side; each `holes` entry is clockwise (same convention as). This
  winding is a writer guarantee, not something the loader re-validates.

`consumed` (top-level) is `[sketch_id, region_id]` dense-id pairs — regions
already extruded into an Object, not to be offered again — sorted ascending.
Every pair must resolve to a real sketch and region; dangling ones are
rejected like any other dangling reference. From v9 the set is derivable
from `objects[].footprints` (it is written redundantly so a reader need
not implement polygon-overlap tests to know what is consumed); a writer
must keep the two consistent — at save time the stored set equals the
derived one.

### 4.7 Guides

A **guide** (`guides[]`) is non-solid construction geometry: never rendered,
never affecting any object's watertightness. Each entry has an id, a `kind`
of `"line"` or `"point"`, and a position `p`. For `"line"`, `p` is the
origin and `dir` (required) is a unit direction; for `"point"`, `dir` is
omitted. A zero/non-normalizable `dir`, or any non-finite coordinate, is a
fatal load error — never defaulted or dropped.

### 4.8 Document tree

`roots` is the ordered list of top-level `NodeRef`s — objects, groups, and
instances with no parent group. Combined with `groups[].members`, recursively
expanding every `"group"` `NodeRef` from `roots` reaches every world-placed
object and instance exactly once. Component-definition members () are
reached only through an instance's `def`, never through `roots` or a group.

### 4.9 Tags

`tags`, wherever it appears on a NODE entry (objects, groups, instances), is
a list of **tag-paths** — each an ordered, root-first list of string
segments. `[["Architecture", "Walls"], ["Level", "L1"]]` means two
independent tags: `Architecture > Walls` and `Level > L1`. Absent/empty
means untagged. Component definitions never carry tags — only their
instances do, since one definition may be placed multiple times under
different tag assignments.

The TOP-LEVEL `tags` array (manifest v5+) is the **tag metadata registry**:
`{"path": [..segments..], "hidden": bool}` entries, sorted by path, one per
*known* tag. It serves two purposes beyond the implicit per-node tags:

- a tag can exist with **no content** (an imported `.skp` layer list
  survives in full, empty layers included);
- `hidden` (default `false`, omitted when false) is the tag's
  **hidden-by-default** flag — content carrying a hidden tag loads
  invisible until the user shows the tag. A `.skp` import maps hidden
  layers here; the UI's tag show/hide toggle persists here.

A node-carried tag path absent from the registry is implicitly visible. A
reader MUST NOT drop content because its tag is hidden — hidden is view
state, not existence.

Separately from tags, each object/group/instance entry MAY carry a
`hidden: true` flag (manifest v6+): the node's own **user-hidden** view
state (a per-node "Hide", covering `.skp` hidden groups/components on
import and the UI's per-node eye toggle). Hiding a group or instance hides
its whole subtree in the UI. The same rule applies: a reader MUST NOT drop
user-hidden content.

## 5. Versioning policy

`format_version` (manifest shape) and `geometry_version` (buffer layout) are
independent counters, each bumped whenever its on-disk shape changes in a
way an older reader would misinterpret: a new required field, a changed
field's meaning, or a reordered/resized binary layout. Purely additive
optional fields () are backward-compatible by construction but still
get a version bump by convention, so a reader can answer "might this file
contain field X" with one integer comparison instead of probing the JSON.

A conforming reader accepts every version from `1` up to the newest it
implements, applying each field's documented default below that field's
introduction, and rejects — with a distinct typed error — version `0` or
anything newer than it implements, never attempting a partial or heuristic
read of an unrecognized version.

A conforming writer always writes the newest version numbers it implements;
this format defines no mechanism for deliberately writing an older version.
