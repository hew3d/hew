# Hew Native File Format (`.hew`)

This document specifies the current `.hew` container format — zip layout,
JSON manifest schema, binary geometry buffer layout, and semantics — in
enough detail for an independent implementation to produce byte-compatible
output and correctly interpret every field, with no access to Hew's source.

Two independent format numbers appear in every file: **manifest format
version `29`**, and **geometry buffer format version `9`**. Both are covered
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
- **Canonical geometry order**: vertices and faces inside a geometry buffer
  are emitted in the topology-derived canonical order of §3.1, never in the
  writer's internal storage order. Internal storage (slot) order is a
  function of an object's mutation history, not its geometry — an undo/redo
  cycle can reallocate storage — and two semantically identical documents
  must serialize to identical bytes.

A reader, by contrast, MUST NOT depend on any of the above: it must accept
entries in any physical order and tolerate any timestamp/permission values.

## 2. Manifest (`manifest.json`)

The manifest is a single JSON object. All arrays below are emitted in
ascending dense-id order (the array index equals the entry's own `id` field).

```jsonc
{
  "format_version": 29,
  "geometry_version": 9,
  "app": "hew",
  "app_version": "1.2.1",
  "document_id": "3f2c9107-5ed4-40a8-806b-1ec297405d83", // v29+, optional;
                                    // omitted = the document has no id yet

  "materials": [
    { "id": 0, "sid": 0, "name": "Red", "color": [220, 50, 40, 255],
      "texture": {                  // omitted entirely for a flat-color material
        "asset": "textures/tex_0.png",
        "format": "png",            // "png" | "jpg"
        "world_size": [1.0, 1.0]    // meters per image tile [width, height], both > 0
      },
      "pbr": {                      // v26+; omitted for a plain material (§4.5)
        "metallic": 0.9, "roughness": 0.25, "normal_scale": 1.0, "occlusion_strength": 1.0
      } }
  ],
  "objects": [
    { "id": 0, "sid": 1, "geometry": "geometry/obj_0.bin", "base_material": 0,
      "name": "Counter_Base",
      "tags": [["Architecture", "Walls"], ["Level", "L1"]],
      "attrs": {                     // v14+; omitted entirely when empty
        "com.example.shelving": { "part_no": "SHLF-204", "load_kg": 35.5 }
      } }
  ],
  "groups": [
    { "id": 0, "sid": 4, "members": [ {"kind":"object","id":0}, {"kind":"group","id":1} ],
      "name": "MyGroup",
      "tags": [["Architecture"]] }
  ],
  "components": [
    { "id": 0, "sid": 5, "name": "MyComponent",
      "members": [ {"kind":"object","id":2}, {"kind":"object","id":3},
                   {"kind":"instance","id":1} ] }  // NodeRefs at v15+; bare
                                                   // object ids at v14-
  ],
  "instances": [
    { "id": 0, "sid": 6, "def": 0,
      "pose": [1,0,0,0, 0,1,0,0, 0,0,1,0],
      "name": "Chair_1",
      "tags": [["Furniture"]],
      "material": 0 }  // v27+, optional; dense `materials[]` id. Omitted =
                       // the placement has no material of its own
  ],
  "sketches": [
    { "id": 0, "sid": 7,
      "plane": [0.0, 0.0, 1.0, 0.0],
      "vertices": [ {"id":0, "p":[0.0, 0.0, 0.0]} ],
      "edges":    [ {"id":0, "from":0, "to":1, "curve":0} ],
      "regions":  [ {"id":0, "outer":[0,1,2,3], "holes":[[4,5,6]],
                     "faceless": true} ], // "faceless" is v21+, optional;
                                          // omitted = the loop has a face
      "curves":   [ {"id":0, "center":[1.0, 2.0, 0.0], "radius":0.75,
                     "kind":"polygon"} ], // v10+; "kind" is v12+, optional
      "owner":    0,  // v13+, optional; dense `components[]` id. Omitted =
                       // world-owned (the only possibility before v13)
      "owner_object": 0, // v17+, optional; dense `objects[]` id. Omitted =
                       // not owned by an Object (the only possibility before v17)
      "owner_group": 0, // v19+, optional; dense `groups[]` id. Omitted =
                       // in no group (the only possibility before v19)
      "tags": [["Site"]] // v20+, optional; omitted = untagged
    },
    { "id": 1, "sid": 12,
      "plane": [0.0, 0.0, 1.0, 0.0],
      "vertices": [ {"id":0, "p":[5.0, 0.0, 0.0]} ],
      "edges":    [ {"id":0, "from":0, "to":1} ],
      "regions":  [ {"id":0, "outer":[0,1,2], "holes":[]} ],
      "detached": true // v23+, optional; omitted = the sketch joins its
                       // plane's arrangement. Never with "owner_object"
    }
  ],
  "guides": [
    { "id": 0, "sid": 8, "kind": "line", "p": [0.0, 0.0, 0.0], "dir": [1.0, 0.0, 0.0],
      "tags": [["Rig"]] }, // v21+, optional; omitted = untagged
    { "id": 2, "sid": 10, "kind": "line", "p": [0.0, 1.0, 0.0], "dir": [0.0, 0.0, 1.0],
      "from": 0.0, "to": 0.0127 }, // v22+, optional; omitted = runs on forever
    { "id": 3, "sid": 11, "kind": "line", "p": [0.0, 2.0, 0.0], "dir": [1.0, 0.0, 0.0],
      "dash": 7239 }, // v24+, optional; omitted = 0x3333 (short dashes)
    { "id": 1, "sid": 9, "kind": "point", "p": [2.0, 3.0, 0.0] }
  ],
  "annotations": [
    { "id": 0, "kind": "linear",
      "a": { "node": {"kind":"object","id":0}, "p": [0.0, 0.0, 0.0] },
      "b": { "node": {"kind":"object","id":0}, "p": [1.0, 0.0, 0.0] },
      "offset": [0.0, -0.5, 0.0], "plane": [0.0, 0.0, 1.0, 0.0] },
    { "id": 1, "kind": "radial", "radial_kind": "diameter",
      "a": { "p": [1.0, 0.5, 0.0] },        // "a" omits "node": a free anchor
      "curve": { "center": [0.5, 0.5, 0.0], "radius": 0.5,
                 "plane": [0.0, 0.0, 1.0, 0.0] },
      "leader_dir": [1.0, 0.0, 0.0] },
    { "id": 2, "kind": "leader",
      "a": { "node": {"kind":"object","id":0}, "p": [0.0, 0.0, 1.0] },
      "offset": [0.2, 0.2, 0.0], "text": "Cut here",
      "detached": true }
  ],
  "roots": [ {"kind":"object","id":0}, {"kind":"instance","id":0} ],
  "tags": [ {"path": ["Architecture", "Walls"], "sid": 10},
            {"path": ["Mock Walls"], "sid": 11, "hidden": true} ],
  "camera": {                        // v13+; omitted entirely if never saved
    "projection": "perspective",     // "perspective" | "parallel"
    "fov_deg": 45.0,
    "eye": [4.0, -6.0, 3.0],
    "target": [0.0, 0.0, 0.0],
    "up": [0.0, 0.0, 1.0]
  },
  "axes": { "origin": [1.0, 2.0, 0.0],           // v13+; omitted entirely at world identity
            "x": [0.0, 1.0, 0.0], "y": [-1.0, 0.0, 0.0] },
  "geo": { "name": "NAD83 / Oregon North", "easting": 2500000.5, "northing": 250000.25,
           "height": 12.75, "x_axis": [0.0, 1.0], "scale": 0.9999,
           "map_unit": "foot", "map_unit_metres": 0.3048 }, // v25+; omitted when the model has no place on a map (§4.16)
  "section_plane": {                 // v16+; omitted entirely when no plane is placed
    "origin": [0.0, 0.3, 0.0],
    "normal": [0.0, 1.0, 0.0],       // unit; points at the side the cut REMOVES
    "active": true
  },
  "scenes": [                        // v16+; omitted entirely when empty; array order = tab order
    { "sid": 42, "name": "Assembled", "description": "Everything, three-quarter view.",
      "camera": { "projection": "perspective", "fov_deg": 45.0,
                  "eye": [4.0, -6.0, 3.0], "target": [0.0, 0.0, 0.0], "up": [0.0, 0.0, 1.0] },
      "hidden_nodes": [4, 6],        // sids of user-hidden objects/groups/instances; absent = not captured
      "hidden_tags": [11],           // sids of hidden tag-registry entries; absent = not captured
      "section": null,               // null = captured, no plane; an object = captured plane; absent = not captured
      "display": { "grid": true, "axes": false, "guides": true } }
  ],
  "attrs": {                         // v14+; the DOCUMENT's own dictionaries
    "com.example.project": { "site": "Bergen" }
  }
}
```

### Field reference

- **`format_version`** (`u32`, required) — manifest schema version. Current: `29`.
- **`geometry_version`** (`u32`, required) — geometry buffer layout version
  used by every entry under `geometry/` in this file. Current: `9`.
  Redundant with the per-buffer version in each buffer's own header (),
  but lets a reader reject a whole file up front without opening buffers.
- **`app`**, **`app_version`** (`string`, required) — free-form writer
  identification, no semantic meaning. A reader MUST NOT branch on these.
  Hew writes the release that saved the file (e.g. `"1.2.1"`). Releases up
  to and including 1.2.0 wrote `"0.1.0"` whatever their own version.
- **`document_id`** (`string`, v29+, optional) — a UUID that identifies the
  document across saves, so data kept outside the file (a plugin's
  per-document storage, provenance) can key on it. Format: exactly 36
  characters, lowercase hexadecimal in the hyphenated `8-4-4-4-12` form
  (`3f2c9107-5ed4-40a8-806b-1ec297405d83`); Hew mints a version 4 UUID
  (RFC 9562), but a reader checks only the textual form. Any other value,
  including uppercase hex, braces, or `null`, is a fatal, typed error. Absent means
  the document has no id yet: every pre-v29 file, and any document never
  saved by a v29 writer. A writer mints the id once, from real randomness,
  the first time it saves the document as `.hew` (a `.skp` save mints it
  too, on the document in memory, though a SketchUp file has no slot for
  it), and never changes it after: a save, a Save As to a new name, and a
  plain file copy all keep it, so two copies of one file carry the same
  id. A Hew Library item made from a whole document leaves it out: the
  item is a separate document, and one opened from it gets its own id on
  its first save. It is file identity, not document content: it is not
  undoable, and it does not take part in the state digest replay
  fixtures pin.
- **`materials`** — the material palette ().
- **`objects`** — every live Object, both world-tree and component-definition
  members ().
- **`groups`** — merge groups: ordered, nestable membership lists ().
- **`components`** — component definitions: flat, ordered object-id sets ().
- **`instances`** — placements of a component definition at a pose ().
- **`sketches`** — first-class 2D sketches ().
- **`guides`** — construction lines and points ().
- **`annotations`** — dimension and leader-text entities ().
- **`roots`** — the document's top-level nodes, in display order ().
- **`tags`** — the tag metadata registry: known tag paths with their
  hidden-by-default flags. Paths are the registry's key: a duplicate path
  is a fatal, typed error at every version, and an empty path is one at
  v14+ (it would consume a stable id nothing can address).
- **`sid`** (`u64`, v14+, required at v14+) — on every `materials[]`,
  `objects[]`, `groups[]`, `components[]`, `instances[]`, `sketches[]`,
  `guides[]`, and `tags[]` entry, and (v19+, required at v19+) on every
  `annotations[]` entry: the entity's **stable id**, the persistent
  identity that survives save/load where dense ids do not (§4.2).
  Globally unique across every kind in one file — a duplicate, or a
  missing `sid` where it is required, is a fatal, typed load error,
  never silently re-minted.
- **`attrs`** (v14+, optional) — attribute dictionaries (docs/agents/HEW_API.md
  §8): on every entity kind that carries a `sid`, and once at the manifest
  top level for the document's own. Shape: `namespace → key → value`,
  namespaces and keys non-empty strings (namespaces reverse-DNS by
  convention; `"hew"`/`"hew.*"` reserved for first-party use), values
  arbitrary JSON whose numbers are all finite — NaN/Infinity cannot occur
  in JSON, and an integer too large for exact storage (beyond 2^63−1) is a
  fatal, typed error, as is a value nested more than 64 levels deep
  (lists and objects count one level each). Omitted entirely when empty, so an attribute-free
  document is byte-identical to one written before the field existed. The
  kernel stores and round-trips dictionaries verbatim and interprets one
  first-party key only: `hew.health` → `surface: true` on an object (a
  definition member included) marks it as a surface someone accepted
  (Model Health); it is an ordinary attribute in the file, and any other
  value means "not marked".
- **`camera`** (v13+, optional) — the camera's working view at last save.
- **`view_style`** (v19+, optional) — how the document asks to be shown:
  its backdrop and axes (§4.15). Absent → Hew's own look.
- **`axes`** (optional object, v13+) — the document-level movable drawing
  axes frame: `origin` (`[f64;3]`), `x` and `y` (`[f64;3]`, unit length and
  mutually perpendicular). The blue axis is never stored — it is always
  `x × y`, recomputed on load — so a persisted frame can never itself
  disagree with its own blue axis. Omitted entirely at world identity (the
  only possibility before this field existed), so an unmoved document's
  manifest is byte-identical to what it would write with no axes concept at
  all. A reader MUST reject rather than repair a present-but-non-orthonormal
  frame (rule 4: no silent geometry repair).
- **`geo`** (optional object, v25+) — the document's coordinate reference
  system (§4.16). Omitted when the model has none.

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
| `sketches[].curves` | 10 | empty list (every curve chain is identity-only, no analytic definition) |
| `sketches[].curves[].kind` | 12 | `"circle"` — the chain's edges are chord facets approximating the stored circle, which is what a `curves[]` entry meant at v10/v11 |
| `sketches[].owner` | 13 | absent — world-owned (the only kind of sketch before v13) |
| `camera` (top-level object) | 13 | absent — the app falls back to today's home framing, exactly as every pre-v13 file already does |
| `axes` (top-level object) | 13 | world identity (origin `[0,0,0]`, `x`=`[1,0,0]`, `y`=`[0,1,0]`) |
| `annotations` (top-level array) | 13 | empty list (no dimensions/leader text) |
| `sid` on `materials[]`/`objects[]`/`groups[]`/`components[]`/`instances[]`/`sketches[]`/`guides[]`/`tags[]` | 14 | pre-v14 upgrade: the loader mints fresh stable ids in dense order (deterministic). At v14+ absence is a fatal, typed error, not a default |
| `attrs` on the same entity kinds, and top-level `attrs` | 14 | empty (no dictionaries) |
| `components[].members` as `NodeRef`s, and `groups[].owner` / `instances[].owner` | 15 | pre-v15: members are bare object dense ids (the only member kind that existed) and every node is world-owned |
| `section_plane` (top-level object) | 16 | absent — no section plane placed |
| `scenes` (top-level array) | 16 | empty list (no Scenes) |
| `sketches[].owner_object` | 17 | absent — no sketch is owned by an Object (the only kind before v17) |
| `sketches[].owner_group` | 19 | absent — no sketch is held by a group (the only possibility before v19) |
| `annotations[].tags` | 18 | empty list (the annotation carries no tag) |
| `camera.parallel_height` (top-level and in `scenes[].camera`) | 19 | the height the perspective view spans at the target: `2 · |eye − target| · tan(fov_deg / 2)` |
| `view_style` (top-level object) | 19 | absent — Hew's own look, the only possibility before this field |
| `scenes[].view_style` | 19 | absent — a Scene restored the editor's toggles alone |
| `annotations[].sid` | 19 | pre-v19 upgrade: the loader mints fresh stable ids in list order, as the v14 upgrade does for the other kinds. At v19+ absence is a fatal, typed error |
| `sketches[].tags` | 20 | empty list (the sketch carries no tag) |
| `sketches[].regions[].faceless` | 21 | `false` (every closed loop has a face, the only possibility before v21) |
| `guides[].tags` | 21 | empty list (the guide carries no tag) |
| `guides[].from`, `guides[].to` | 22 | absent: the line runs on forever both ways, the only possibility before v22 |
| `sketches[].detached` | 23 | `false` (every sketch joins its plane's arrangement, the only possibility before v23) |
| `camera.image_width`, `camera.aspect_ratio` (top-level and in `scenes[].camera`) | 24 | absent: unset, the only possibility before v24 |
| `view_style.section_cut_color`, `section_cut_width`, `section_fill`, `section_fill_color`, `active_section_color`, `guides_color`, `guides_hidden` (top-level and in `scenes[].view_style`) | 24 | absent: unset, Hew's own look |
| `guides[].dash` | 24 | `0x3333` (short dashes, how every guide was drawn before v24) |
| `geo` (top-level object) | 25 | absent: the model has no geo-reference, the only possibility before v25 |
| `materials[].pbr` | 26 | absent: a plain material (no metal, fully rough, no maps), the only possibility before v26 |
| `instances[].material` | 27 | absent: the placement has no material of its own, the only possibility before v27 |
| `document_id` (top-level string) | 29 | absent: the document has no id yet; the next save mints one |

Manifest v28 adds no manifest field: it marks geometry buffer v9 (face
and edge attribute dictionaries, §3), so a v28 manifest names
`geometry_version` 9.

`document_id` (v29) is version-gated the usual single direction: a file
declaring a version older than 29 that carries it is malformed for its own
declared version and MUST be rejected. At v29+ it is optional, but a
present one must have the exact textual form the field reference gives.

Four fields land in the same v13 bump (four efforts in flight at once) and
are version-gated the same way the retired fields below are (just in the
opposite direction — introduced, not retired): the gate is on the declared
`format_version`, not on the field's presence. Each is gated independently
by its own presence, not by the shared version number alone — a v13 file
with any subset of them, or none, is perfectly ordinary.

- A file that declares a version **older than 13** but still carries an
  `owner` field is malformed for its own declared version and MUST be
  rejected with a typed error, never honored — a pre-v13 writer could never
  have produced it, and silently attaching a sketch to a definition on the
  strength of a field that version claims not to have would be exactly the
  silent repair this format refuses everywhere else.
- Likewise, it is malformed for a file to declare `format_version < 13` and
  STILL carry a `camera` block (no legitimate writer older than v13 ever
  emitted one), and a conforming reader MUST reject such a file with a typed
  error rather than accept "an early camera" — reject-not-repair, the same
  posture the `consumed`-list check below applies in the opposite
  direction.
- Likewise, a file that declares `format_version < 13` and still carries an
  `axes` key is malformed for its own declared version and is rejected, not
  repaired.
- The top-level `annotations` field is version-gated the same way, for the
  same reason: a file declaring a version older than 13 that still carries
  `annotations` is malformed for its own declared version and MUST be
  rejected rather than silently loaded.
- **`sid` (v14) is gated in both directions.** A file declaring a version
  older than 14 that carries any `sid` field is malformed for its own
  declared version and MUST be rejected (no pre-v14 writer ever emitted
  one). A file at v14 or newer must carry a `sid` on **every** entity entry
  of the kinds listed in the field reference, all globally unique; a
  missing or duplicated `sid` is a fatal, typed error — a reader MUST NOT
  re-mint or renumber to "fix" the file (reject-not-repair). The mint
  counter itself is deliberately **not** serialized: a loader resumes
  minting at `max(all sids) + 1`, so ids of entities that never reached
  the file (an undone creation, say) are simply never observed.
- **`attrs` (v14)** is version-gated the usual single direction: a file
  declaring a version older than 14 that carries any `attrs` field is
  malformed for its own declared version and MUST be rejected. At v14+ the
  field is optional (absence means no dictionaries), but a present one
  must satisfy the field reference's shape rules — an empty namespace or
  key string, or an unrepresentable number, is a fatal, typed error.

Three fields existed only in older versions and are **retired at v11**: the
top-level `consumed` list (v1–v10), `objects[].source` (v8 only), and
`objects[].footprints` (v9/v10). They stored the sketch–solid claim data of
the footprint consumption model; the current model allows re-extruding
occupied ground outright — solids interpenetrate freely — so there is
nothing to store, and writers from v11 on emit none of them.

A current reader (v11 and later) treats them differently on an older file:

- **`consumed`** is honored ONE final time, then discarded — and only in
  files whose declared `format_version` is **older than 11** (the gating
  is on the declared version, not on the field's presence). Older files
  persisted extruded outlines as ordinary sketch edges (hidden at runtime,
  not deleted); the reader applies the current consumption model to the
  stored index retroactively — each consumed region's exclusive
  scaffolding is deleted on load (an edge shared with a surviving region
  survives, exactly as at extrusion time), and a sketch the deletion
  empties is removed with it. Each pair must resolve to a real sketch and
  region; a dangling pair is a fatal, typed dangling-reference error like
  any other. The loaded document therefore looks exactly as it did in the
  old build — nothing previously consumed resurrects — and a resave emits
  the current version with no claim fields. A file that declares `format_version`
  **11 or newer** and still carries a `consumed` field is malformed for
  its own version and MUST be rejected with a typed error — never acted
  on and never ignored (reject-not-repair; deleting sketch geometry on
  the say-so of a retired field would be silent repair).
- **`objects[].footprints`** and **`objects[].source`** are ignored
  entirely: they carried the stored consumption claims of the retired
  footprint model. They are not validated, not resolved, and change nothing
  about the loaded document.

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

### 3.1 Layout (current version, `9`)

```
offset  type                field
0       u8[4]               magic = "HEWG" (0x48 0x45 0x57 0x47)
4       u32                 version (currently 9)
8       u8                  watertight: 0 = Open (leaky), 1 = Watertight
9       u32                 base_material id (0xFFFFFFFF = none)
13      u8                  imported flag: 0 = strict native tolerance, 1 = imported tolerance
14      u32                 vertex_count (N)
18      f64[3] * N          vertex positions, one (x, y, z) triple per vertex, in canonical order (below)
...     u32                 face_count (F)
        --- repeated F times, in canonical face order (below): ---
        u32                 face material id (0xFFFFFFFF = none)
        u8                  uv_frame flag: 0 = no UV frame, 1 = UV frame follows
        f64[8]              uv_frame, present only when the flag above is 1:
                             s.x s.y s.z  t.x t.y t.z  u0 v0
        u8                  surface flag: 0 = plain planar face, 1 = cylinder reference follows
        f64[7]              surface, present only when the flag above is 1:
                             axis_point.x .y .z  axis.x .y .z  radius
        u32                 outer_count (K)
        u32[K]              outer-loop vertex indices (0-based into the vertex table, in loop order)
        u32                 hole_count (H)
        --- repeated H times: ---
        u32                 hole_vertex_count (J)
        u32[J]              hole-loop vertex indices
        u8                  soft_edges flag (v6): 0 = no edge of this face is soft, 1 = per-edge flags follow
        --- present only when the soft_edges flag is 1: ---
        --- one entry per outer edge (K entries), then per hole edge (J per hole, in hole order): ---
        u8                  soft flag: 0 = hard edge, 1 = soft (a smooth-sweep facet seam; renderers smooth shading across it and suppress its line)
        u8                  edge_curves flag (v5): 0 = no edge of this face carries an analytic circle, 1 = per-edge claims follow
        --- present only when the edge_curves flag is 1: ---
        --- one entry per outer edge (K entries), then per hole edge (J per hole, in hole order): ---
        u8                  edge curve flag: 0 = plain edge, 1 = circle claim follows
        f64[4]              edge curve, present only when the flag above is 1:
                             center.x .y .z  radius
        u8                  edge_polygons flag (v7): 0 = no edge of this face is a regular polygon's side, 1 = per-edge claims follow
        --- present only when the edge_polygons flag is 1: ---
        --- one entry per outer edge (K entries), then per hole edge (J per hole, in hole order): ---
        u8                  edge polygon flag: 0 = plain edge, 1 = polygon claim follows
        f64[4]              edge polygon, present only when the flag above is 1:
                             circumcircle center.x .y .z  radius
        u8                  back paint flag (v8): 0 = the back shows what the front shows, 1 = a back material id follows
        u32                 back material id, present only when the flag above is 1
        u8                  face_attrs flag (v9): 0 = no dictionary, 1 = a dictionary follows
        u32                 byte length (L), present only when the flag above is 1
        u8[L]               the face's attribute dictionary as UTF-8 JSON, present only when the flag above is 1
        u8                  edge_attrs flag (v9): 0 = no edge of this face carries a dictionary, 1 = per-edge entries follow
        --- present only when the edge_attrs flag is 1: ---
        --- one entry per outer edge (K entries), then per hole edge (J per hole, in hole order): ---
        u8                  edge dictionary flag: 0 = none, 1 = a dictionary follows
        u32                 byte length (L), present only when the flag above is 1
        u8[L]               the edge's attribute dictionary as UTF-8 JSON, present only when the flag above is 1
```

A face or edge **attribute dictionary** (v9) is the same shape as the
manifest's `attrs` (§2 field reference): a JSON object of namespace →
key → value, compact, with object keys in ascending byte order (a
conforming writer emits it with no whitespace, which makes equal
dictionaries byte-equal). It is client data the kernel stores and never
interprets (docs/agents/HEW_API.md §8). An entry is written only when the
element carries a non-empty dictionary; an empty one is the 0 flag. Like
the other per-edge blocks, an edge's dictionary is written by every face
the edge bounds, at the entry for that edge, and the two entries of a
shared edge must be identical: both absent, or equal dictionaries. A plain
solid grows by the two flag bytes per face.

A face's back paint (v8) is the material on its back side: a `.skp` face
can be painted on each side. Renderers draw it where the back can be seen
(an open shell, a cut); it never changes shape or watertightness. Any flag
value other than 0 or 1 is malformed and MUST be rejected.

An edge polygon claim (v7) marks an edge as a side, or a piece of a side,
of a regular polygon drawn on the face, naming the polygon's circumcircle.
The sides are the geometry, so the claim implies no surface or smoothing;
it lets an editor show the polygon as one shape with a center. Both faces
of a shared edge store it and must agree within 1e-9 m.

Each face's outer loop and each of its hole loops is a closed polygon: the
last index does not repeat the first (the loop is implicitly closed by
wrapping back to index 0 of that loop).

Every loop is **simple**: no vertex index appears twice in one loop. The
loops of one face may **share vertices at isolated points** — a hole
touching the face's outer loop at one vertex (a circle drawn tangent to the
face's edge), or two holes touching at one vertex — but never an edge: no
two loops of one face traverse the same pair of vertices, in either
direction. A shared vertex is one vertex-table entry referenced from each
loop through it. This needed no version bump: every earlier reader rebuilt
such a face from its index loops and its validator accepted it (a hole was
checked only for lying inside the outer loop and not crossing it, and
touching at a vertex is not a crossing), and no earlier writer emitted a
loop repeating a vertex or an edge shared between loops of one face, so the
stricter checks below reject nothing a conforming writer produced.

**Canonical order.** A conforming writer derives the emission order from the
geometry itself (see "Determinism"), as follows. Positions are compared
lexicographically by coordinate — `x`, then `y`, then `z` — using a total
order over the raw f64 bit patterns (IEEE 754 `totalOrder`; Rust
`f64::total_cmp`), never a quantized or epsilon comparison. Then:

- every loop (outer and hole) is rotated to start at the vertex that makes
  its position sequence lexicographically smallest; winding is preserved —
  only the starting vertex changes;
- a face's hole loops are sorted among themselves by their (rotated)
  position sequences;
- faces are sorted by their rotated outer-ring position sequence, then hole
  count, then hole sequences, then material id, then UV frame (frameless
  faces first; frames compared elementwise `s.x s.y s.z t.x t.y t.z u0 v0`
  under the same f64 total order), then the per-face analytic surface (v4;
  surface-less faces first, then by `axis_point.x y z axis.x y z radius`
  under the f64 total order), then the per-edge circle claims (v5; the outer
  loop's claim sequence in canonical vertex order, then each hole's, each
  compared by length then entry — claim-less before claim-bearing, then by
  `center.x y z radius`), then the per-edge polygon claims (v7; compared
  the same way), then the face's attribute dictionary (v9; absent before
  present, present compared as its emitted JSON bytes, lexicographically),
  then the per-edge dictionaries (the outer loop's entry sequence, then
  each hole's, absent before present, present by bytes). Coincident faces
  on disjoint shells can tie on geometry alone, so every emitted payload —
  ring, holes, material, UV frame, surface, edge claims, and dictionaries
  — participates in the key;
- vertices are numbered by first appearance while walking the faces in that
  order (each face's outer loop, then its holes); a vertex several loops
  share (a touching hole) takes the number of its first appearance and is
  referenced by that number from every later loop; a vertex referenced by no
  loop — which valid topology does not produce — is appended at the end in
  position order. Loops being simple, each loop's rotation start is unique,
  so the order is well defined with shared vertices too.

A reader MUST NOT depend on this order (any vertex/face order decodes to
the same object); it exists so that semantically identical documents
serialize to identical bytes regardless of the writer's mutation history.

`uv_frame`, when present, is an oriented planar UV mapping: for a world
point `p` on the face's plane, `uv = (s·p + u0, t·p + v0)`, where `s`/`t`
are (not necessarily unit or orthogonal) world-space vectors and `u0`/`v0`
are scalar offsets. A face with no `uv_frame` instead uses its material's
`world_size` planar projection when textured () — mutually exclusive
paths, selected by this flag.

`surface` (v4+), when present, is the face's **analytic surface
reference**: the face is a planar chord facet of the infinite right
circular cylinder through `axis_point` along unit `axis` with the given
`radius` — durable metadata over the faceted geometry (the extruded side
walls of arc/circle profiles carry it), never a replacement for the
polygonal face itself. The face's plane is parallel to `axis`, at most
`radius` from it, and every face vertex lies within `radius` of the axis
line; angular/axial extent is derived from the vertices, never stored. A
reader that ignores the field loses only analytic metadata (exact
center/axis/radius for snapping, smooth shading, re-export), never shape
or watertightness.

The per-face **edge-curve claims** (v5+) are the solid-edge mirror of the
sketch-edge curve chains: an edge marked with a circle is a chord facet of
the infinite circle through `center` with the given `radius` (each endpoint
within tolerance of `radius` from `center`). They carry a circle drawn
on a solid face (lowered to topology by the drafting engine, not yet pushed) so a later push-through can
re-attribute the tunnel walls as cylinder surfaces. The claims are stored
per face parallel to that face's loop edges; a shared edge appears in both
incident faces and carries the same claim in each. The leading `edge_curves`
flag is `0` for the overwhelmingly common face that carries no such edge
(one byte), so plain solids grow by exactly one byte per face. A reader that
ignores the block loses only the drawn circle's analytic identity, never shape.

### 3.2 Field availability by version

A conforming writer always writes one buffer version per file, matching the
manifest's `geometry_version`; a reader should nonetheless decode each
buffer by its own header `version` field, not by trusting the manifest
value uncritically. These fields are version-gated; every other field is
present at every version:

| Field | First appears at buffer `version` | Byte layout when absent | Effective value when absent |
|---|---|---|---|
| header `imported` flag (offset 13) | 3 | byte is not present at all — `vertex_count` begins one byte earlier | `0` (strict native tolerance) |
| per-face `uv_frame` flag + payload | 2 | flag byte and payload are not present at all — `outer_count` follows the material id directly | no UV frame |
| per-face `surface` flag + payload | 4 | flag byte and payload are not present at all — `outer_count` follows the `uv_frame` block directly | no analytic surface |
| per-face `edge_curves` flag + payload | 5 | flag byte and payload are not present at all — the next face's material id (or the buffer's end) follows the last hole loop directly | no edge carries an analytic circle |
| per-face `soft_edges` flag + payload | 6 | flag byte and payload are not present at all — the `edge_curves` flag (v5) follows the last hole loop directly | no edge is soft |
| per-face `edge_polygons` flag + payload | 7 | flag byte and payload are not present at all — the next face's material id (or the buffer's end) follows the `edge_curves` block directly | no edge is a polygon's side |
| per-face back paint flag + id | 8 | flag byte and id are not present at all — the next face's material id (or the buffer's end) follows the `edge_polygons` block directly | no face has a painted back |
| per-face `face_attrs` and `edge_attrs` blocks | 9 | neither flag byte nor any payload is present — the next face's material id (or the buffer's end) follows the back paint block directly | no face or edge carries a dictionary |

All gates shift subsequent byte offsets, so a reader must branch on the
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
  vertices (`outer_count`/`hole_vertex_count < 3`), a `watertight`/
  `uv_frame`/`imported`/`surface`/`edge_curves` byte outside its 0-1 range,
  a `surface` payload whose `axis` is not unit length or whose `radius` is
  not finite and strictly positive, or an edge-curve payload whose `radius`
  is not finite and strictly positive — is a fatal error reported with the
  byte offset and what was wrong. Never a panic, never best-effort
  recovery. (After rebuild, the validator additionally rejects an edge-curve
  claim whose endpoints are not within tolerance of `radius` from `center`,
  exactly as it rejects a lying `surface`.)
- **Attribute dictionaries (v9).** A `face_attrs`, `edge_attrs`, or
  per-edge dictionary flag outside 0-1, a length running past the end of
  the buffer (truncation), or a present dictionary that is not valid
  UTF-8 JSON of the `attrs` shape is a fatal error. The shape is held as
  strictly as a live write: a JSON object of at least one namespace; every
  namespace a non-empty string holding at least one key; every key a
  non-empty string; every value representable (finite numbers, integers
  within `i64`, nesting at most 64 levels, the manifest's `attrs` rules).
- **Shared-edge agreement (v9).** The two faces of a shared edge must
  write the same dictionary entry for it (both absent, or equal
  dictionaries). A disagreement is a tampered or corrupt file, rejected
  as such, never resolved by which face was read last.
- **Face plane recomputation.** Planes are *not* stored; a reader must
  recompute each face's best-fit plane from its outer-loop vertices (Newell's
  method over the loop in the order the writer emits it, rotated to its
  canonical start) and reject a loop that doesn't span a plane — a Newell
  vector (twice the enclosed area) shorter than 1e-12 m² — or whose vertices
  stray from it beyond tolerance (gated by `imported`). The kernel's
  validator holds every live face to the same predicate, so no operation
  commits a face, however small, that a saved file could not reopen. This is safe because a baked reflection reverses every
  face's loops (), so the recomputed plane's normal always matches how the
  geometry was authored.
- **Watertightness cross-check.** The stored `watertight` byte must equal
  what the reader's own topology reconstruction computes — a mismatch is a
  fatal error (cheap tamper detection), never silently corrected.
- **Full topology validation.** The normal topology validator (manifold
  edges, closed face loops, no orphan geometry, every loop simple, no edge
  with both sides on one face) must pass before a decode counts as
  successful.

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

The identity that IS stable is the **`sid`** (v14+): a `u64` minted once
when its entity is created, carried on the entity for the document's whole
life, and written with it on every save. Two saves of the same document
renumber their dense ids freely but agree on every `sid`, so a client that
must remember "this object" across independent loads caches the `sid` and
re-resolves it after each load. Stable ids are globally unique across
every entity kind in one file (an object and a material never share one),
are never reused within a document's lifetime, and survive undo/redo —
undoing a deletion restores the entity's original `sid`. The sketch-local
dense ids of vertices/edges/regions have no stable counterpart: sketch
sub-entities remain addressable only relative to their sketch and only
within one load (docs/agents/HEW_API.md §5.1 builds its public sub-entity ids on
exactly that contract).

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
optional `name`, and optional `tags` ().

An object's faces and edges may each carry an attribute dictionary
(geometry buffer v9, §3.1), stored with the geometry rather than in the
manifest because faces and edges have no stable id there.

An object needs no stored relationship to the sketch it was extruded from:
extrusion deletes the profile's scaffolding from the sketch (the outline
became the solid's base face), and re-extruding occupied ground is simply
allowed — Hew's solids interpenetrate freely, so no claim data is stored
or derived. Older files' `footprints` (v9/v10) and `source` (v8) fields
are ignored on load (see the retired-fields note in the compatibility
section).

Every object id appears in **exactly one** structural position: either a
member of exactly one component definition (`components[].members`), or
reachable from the world tree (directly in `roots`, or nested inside a
`groups[].members` chain) — never both at once.

A transform baked into a buffer's vertex positions may be a reflection
(determinant negative). A baked reflection reverses every face's loops, so
the loops still wind counter-clockwise seen from the outward normal —
this is what lets the geometry decoder recompute face-plane orientation
from winding alone (). A mirrored placement may also exist as a component
**instance** pose.

### 4.4 Groups, components, instances

- **Groups** (`groups[]`) are a plain, ordered membership list of
  `NodeRef`s — objects or other groups, nesting unrestricted — plus an
  optional name and tags. A group has no geometry or transform of its own.
- **Component definitions** (`components[]`) are an ordered list of member
  `NodeRef`s (v15+; pre-v15 files carry bare object dense ids, the only
  member kind that existed), plus an optional name. Members may be
  objects, groups, or **instances of other definitions** — nesting. The
  definition graph MUST be acyclic, depth-bounded, and width-bounded: a
  reader rejects a file whose definitions reach themselves, nest beyond
  64 levels, or whose expansion exceeds 1,000,000 leaf placements (per
  definition, and summed over the world instances) — all with typed
  malformed-manifest errors. The width bound exists because a DAG
  multiplies: twenty levels of two instances each is a million
  placements. Writers cannot produce such a file through the ops (the
  same bound refuses growth at every op), so the reader bound only ever
  rejects hand-crafted input. A definition is never placed
  directly, carries no transform/tags/position, and exists purely to be
  instanced. Its shared geometry may also include not-yet-extruded
  sketches — those are found by scanning `sketches[].owner` (§4.6, v13+)
  for this definition's dense id, not listed on the `components[]` entry
  itself.

  **Definition-owned nodes (v15+).** A group or instance that is part of a
  definition's subtree carries an `owner` field naming that definition's
  dense id — on DIRECT members and on every group/instance nested beneath
  a member group alike. Ownership is redundant with tree position by
  design (cheap tamper detection, like the watertight byte): a reader MUST
  derive ownership from the member lists and reject any entry whose
  declared `owner` disagrees with the derivation — including an
  owner-declaring node no definition's subtree reaches, a member whose
  `owner` is absent, and a node reachable from two definitions. A
  definition-owned node never appears in `roots` and is reached only
  through its definition's instances; its coordinates (an instance
  member's `pose` included) are in the OWNING definition's local frame.
  Version-gated both directions: a pre-v15 file carrying a `NodeRef`
  member or any `owner` field is malformed for its declared version, and a
  v15+ file carrying a bare-integer member is a nonconforming writer's
  output — both rejected, never guessed at.
- **Instances** (`instances[]`) place one component definition (`def`, a
  dense component id) at a **pose**: a row-major 3×4 affine matrix
  `[m00,m01,m02,tx, m10,m11,m12,ty, m20,m21,m22,tz]`, read as `world_point =
  linear * local_point + translation` (`linear` the 3×3 block, `translation`
  the vector `(tx,ty,tz)`). Unlike baked object geometry, a pose **may** be
  non-uniformly scaled, and may be reflective without turning any face
  over, since poses are not baked into the definition's geometry. Each instance carries its own optional name
  (falling back to the definition's name if absent) and optional tags.

  An instance may carry its own `material` (v27+, optional): a dense
  `materials[]` id that MUST name an entry of this file (an out-of-range
  id is a fatal, typed error). Definition-owned instances carry one too.
  It colors only what nobody painted. A face of a definition member
  resolves its material in this order: the face's own `material`; else
  its object's base material; else, walking the instances that place the
  member from the innermost outward, the first one that has a
  `material`; else the renderer's default. A painted face or a painted
  object base therefore keeps its color in every placement, and a nested
  instance's material wins over its outer instance's. A file declaring
  `format_version < 27` that carries an instance `material` is malformed
  for its declared version and MUST be rejected, never honored.

### 4.5 Materials and textures

Materials are referenced by faces (front and back), object bases, and
instances (§4.4). Each `materials[]` entry has an id, an informational `name`, and a `color`
(`[r, g, b, a]`, each `0-255`). A material may optionally carry a `texture`:
`asset` (the zip path of the verbatim image bytes), `format` (`"png"` or
`"jpg"`, matching the asset's actual encoding), and `world_size` (`[width,
height]` meters per image tile). A textured face with no per-face
`uv_frame` () is planar-projected and tiled at `world_size`; a face
that does carry a `uv_frame` ignores `world_size` and uses the frame's
affine mapping instead. A material with no `texture` renders as a flat
`color` fill.

A material may also carry `pbr` (v26+, optional, omitted when plain): its
physically based surface. `metallic` and `roughness` (f32, 0 to 1) and
`occlusion_strength` (f32, 0 to 1) and `normal_scale` (f32, 0 and up) are
required inside it; `metallic_map`, `roughness_map`, `normal_map` and
`occlusion_map` are each optional `{ "asset", "format" }` objects naming a
zip path (`textures/pbr_<material id>_<slot>.png`) of verbatim image bytes
and their codec (`"png"` or `"jpg"`). A map uses the same planar projection
and tile size as the material's base `texture`. The plain surface is
`metallic` 0, `roughness` 1, both strengths 1 and no maps; a writer MUST
omit `pbr` for it. A reader MUST reject, never repair, a value outside the
ranges above, an unknown codec, a key not listed here, and a manifest
declaring `format_version < 26` that carries `pbr` at all.

A material the user deletes from the palette is tombstoned in-session
(its id stays valid for undo) but never written out: `save()` simply
omits it from `materials[]`, so deletion carries no format change — a
reader sees exactly the same shape it always has, one entry shorter.

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
  first-appearance order over the edge list — walking `edges[]` in order,
  the first occurrence of each distinct `curve` value must be exactly one
  greater than the previous maximum (starting at 0). This is a writer
  guarantee AND a reader obligation: a reader MUST reject a first
  appearance that skips ahead as malformed, never gap-fill phantom chains
  (gap-filling would also defeat the dangling-`curves[]`-entry check).
  Chain ids carry no geometry of their own (that lives in `curves[]`,
  v10+) — a reader that ignores them loses only selection grouping, never
  shape.
- `regions[]` — `{id, outer, holes, faceless?}`: a closed planar
  polygon-with-holes. `outer` is a vertex-id cycle, counter-clockwise from
  the plane normal's side; each `holes` entry is clockwise (same
  convention as). This winding is a writer guarantee, not something the
  loader re-validates. Each ring is simple; a hole may share vertices with
  the outer ring or another hole, as a solid face's loops may (§3.1). A file
  written before rings were split at touch points may hold a ring that
  visits one vertex twice (a circle tangent to a rectangle's side, traced
  as one walk); a reader loads it as written, and the sketch's next edit
  re-traces it into simple rings. `faceless` (v21+, optional, omitted when false)
  marks a loop of edges with no face: what SketchUp has where a face was
  erased from its edges or where edges alone were drawn (a wire frame, a
  light rig). Such a region is not filled, not picked as a face and not
  extruded, and a `.skp` save back writes its edges alone; drawing over
  one of its edges gives it its face, and a region a drawing step brings
  about has one. A manifest declaring `format_version < 21` that sets
  `faceless` MUST be rejected.
- `curves[]` (v10+, optional) — `{id, center, radius, kind?}`: the
  **analytic definition** of one curve chain — the exact circle, in the
  sketch plane, that the chain's edges relate to. `id` is the same dense
  per-sketch curve id `edges[].curve` references. Sparse: a chain with no
  entry is identity-only (selection grouping without geometry — every
  pre-v10 chain). A reader MUST reject an entry whose `radius` is not
  finite and strictly positive, an `id` no edge references, or a duplicated
  `id` — like any other dangling reference. `center` is a writer guarantee
  to lie on `plane` (like vertex positions, not re-validated on load). A
  reader that ignores `curves` loses only the analytic metadata (exact
  center/radius), never shape.

  `kind` (v12+, optional) says what the circle **claims** about the chain,
  and the two claims are not interchangeable:

  - `"circle"` (the default when `kind` is absent, and the only meaning
    `curves[]` had at v10/v11) — the chain's edges are chord facets
    *approximating* the circle. The circle is the truth and the facets are
    an artifact, so the circle's centre, its four quadrant points and its
    tangents are all real points of the drawing, offsetting the chain
    produces a concentric arc, and extruding a profile bounded by it sweeps
    a cylindrical surface.
  - `"polygon"` — the chain is a regular polygon and its edges **are** the
    geometry; the stored circle is only their circumcircle. The one thing
    it contributes is the **centre** the author placed. A reader must not
    derive quadrant points, tangents, concentric offsets or a swept
    cylindrical surface from it: those describe the circumcircle, not the
    polygon, and none of them lies on the shape.

  A writer emits `kind` only for `"polygon"`; a circle's entry is written
  exactly as v10/v11 wrote it, so a document containing no polygon produces
  a byte-identical `curves[]` across the version bump. A reader MUST reject
  any other `kind` string rather than fall back to `"circle"` — guessing
  would hand a shape an analytic identity it does not have. A v11 reader
  handed a v12 file loses the distinction (it reads a polygon as a circle),
  which is why the manifest version moves.

- `owner` (v13+, optional) — the dense `components[]` id of the component
  definition whose shared geometry this sketch belongs to, in DEFINITION-
  LOCAL coordinates (its `plane` is expressed in that definition's own
  frame, exactly like a definition-member object's geometry — ). Absent
  means world-owned, expressed in world coordinates: the only kind of
  sketch before v13. A definition-owned sketch is never placed directly; it
  reaches the scene only through that definition's instances, at each
  instance's pose, exactly like a member object. A reader MUST reject an
  out-of-range `owner`, like any other dangling reference.
- `owner_object` (v17+, optional) — the dense `objects[]` id of the Object
  this sketch belongs to: drawing over that object's faces that is not face
  topology, which rides with the object (moves with it, lives and dies with
  it). Absent means the sketch belongs to no Object: the only kind of
  sketch before v17. The sketch lives in its owner's frame, so the owner
  object's definition must be exactly the sketch's own `owner`: a sketch
  owned by a world object has no `owner`, and a sketch owned by a member of
  component `C` has `owner` = `C` (an `owner_object` naming a definition
  member while `owner` is absent, or naming a member of a different
  component, is inconsistent). A reader MUST reject an out-of-range
  `owner_object` like any other dangling reference, MUST reject an
  `owner_object` whose object's definition disagrees with the sketch's
  `owner` as a malformed manifest, and MUST reject a file declaring
  `format_version < 17` that carries the field at all (reject-not-repair: a
  pre-v17 writer could not have produced it). A sketch whose owner object
  is not saved (deleted, or consumed by a boolean) is not saved either; a
  writer never emits an `owner_object` that names an unsaved object.
- `detached` (v23+, optional, omitted when false) — the sketch is a
  **detached shape**: drawing that takes part in no plane arrangement.
  Drawing made later on the same plane does not split or merge with it,
  moving it onto other drawing or onto a solid's face does not join it,
  and extruding one of its regions always makes a new Object. It is a
  free sketch in every other respect (`owner`, `owner_group`, `tags`
  apply as usual). A detached sketch is never object-owned: a reader
  MUST reject `detached` together with `owner_object` as a malformed
  manifest, and MUST reject a manifest declaring `format_version < 23`
  that sets `detached` (reject-not-repair: a pre-v23 writer could not
  have produced it).

- `owner_group` (v19+, optional) — the dense `groups[]` id of the group the
  sketch is drawn in: a `.skp` group of edges, or lines drawn while
  editing a group. The sketch keeps its frame — a group inside component
  `C` holds sketches with `owner` = `C`, a world group holds world
  sketches — so a reader MUST reject an `owner_group` whose group's
  definition disagrees with the sketch's `owner`, and a dangling one like
  any other dangling reference. Such a sketch is not drawn while its
  group (or a group above it) is user-hidden or carries a hidden tag, is
  deleted with the group, and is released by an ungroup. A `.skp` save
  back writes it into the group's own list.

- `tags` (v20+, optional, omitted when empty) — the tag paths the sketch
  carries, root-first, as a node's `tags` do (§4.10). A sketch with a
  hidden tag, or a tag below a hidden one, is not drawn, on top of what
  its group hides. A `.skp` edge that bounds no face takes its layer's
  tag (edges of one plane on different layers open as different
  sketches), and a save back writes the first tag as the edges' layer. A
  tag rename or delete rewrites these lists as it does a node's. A
  manifest declaring `format_version < 20` that carries a non-empty
  `tags` on a sketch, or any sketch with an empty tag path or path
  segment, MUST be rejected.

### 4.7 Guides

A **guide** (`guides[]`) is non-solid construction geometry: never rendered,
never affecting any object's watertightness. Each entry has an id, a `kind`
of `"line"` or `"point"`, and a position `p`. For `"line"`, `p` is the
origin and `dir` (required) is a unit direction; for `"point"`, `dir` is
omitted. A zero/non-normalizable `dir`, or any non-finite coordinate, is a
fatal load error — never defaulted or dropped.

`tags` (v21+, optional, omitted when empty) are the tag paths the guide
carries, root-first, as a node's `tags` are (§4.10). A guide with a hidden
tag, or a tag below a hidden one, is in the document and in a save but is
not drawn or snapped to, so a Scene that hides the tag hides the guide. A
`.skp` guide opens tagged with its layer and then each container's layer
above it, and a save back writes the first tag as its layer. A tag rename
or delete rewrites these lists as it does a node's. A manifest declaring
`format_version < 21` that carries a non-empty `tags` on a guide, or any
guide with an empty tag path or path segment, MUST be rejected.

`from` and `to` (v22+, optional, `"line"` only) are the stretch of the
line that exists, in meters along `dir` from `p`. An end left out runs on
forever, so a line with neither is a whole line, one with `from` alone is
a ray, and one with both is a segment. SketchUp's Tape Measure makes rays
and segments and shows only that stretch; a `.skp` guide opens with its
own, a save back writes it as the guide's bounds, and only that stretch
is drawn and snapped to. A guide Hew draws is a whole line and writes
neither. A manifest declaring `format_version < 22` that carries either
on a guide, a `"point"` that carries either, a non-number, or `to` not
past `from` by more than the merge tolerance, MUST be rejected.

`dash` (v24+, optional, `"line"` only) is how the line is drawn: a 16-bit
on/off mask (`0..=65535`), least-significant bit first, one bit per fixed
length along the line, so `0xFFFF` (65535) is solid, `0x3333` (13107)
short dashes, `0x8888` (34952) dotted, `0xF0F0` (61680) long dashes and
`0x1C47` (7239) dash, dot, dash. Any mask is valid and round-trips. It is
omitted at the default `0x3333`, which is also what a guide Hew draws and
what a pre-v24 file's guides have. A `.skp` guide opens with its stipple
and a save back writes the guide's dash (a fresh write likewise). A
manifest declaring `format_version < 24` that carries `dash`, or a
`"point"` that carries it, MUST be rejected.

### 4.8 Annotations

An **annotation** (`annotations[]`, v13+) is a dimension or leader-text
entity: like a guide, non-solid and never affecting any object's
watertightness, but a document entity with additional state a guide has
none of (an editable text override, and a `detached` flag). Every entry has
an id and a `kind` of `"linear"`, `"radial"`, or `"leader"`; which of the
remaining fields are present depends on `kind`, following the same
one-flat-DTO convention as a guide's `"line"`/`"point"` shape:

- **`"linear"`** — a straight dimension between two anchors, `a` and `b`
  (see `Anchor` below), with a placement `offset` (`[x,y,z]`, the drag-out
  vector from the `a`–`b` line to where the dimension line sits) and a
  `plane` (`[nx,ny,nz,offset]`, the same plane-encoding convention a sketch
  uses (), the plane the dimension and its extension lines are drawn in).
  Optional `text_override` replaces the app-computed measurement string.
- **`"radial"`** — a radius/diameter dimension. `a` is the single anchor (the
  point on the measured circle/arc the leader points to). `radial_kind` is
  `"radius"` or `"diameter"` — a display choice only, it does not change
  `curve`. `curve` is the analytic circle/arc captured at creation:
  `{center: [x,y,z], radius: (> 0), plane: [nx,ny,nz,offset]}`. `leader_dir`
  is the leader line's direction from `a`. Optional `text_override` as
  above.
- **`"leader"`** — free-form leader text. `a` is the anchor the leader
  points to, `offset` places the text relative to it, and `text` (required)
  is its content.

**`Anchor`** (`a`, and `b` for a linear dimension) is `{node?, p}`: `p` is
the anchor's point in world space; `node` (optional) is a `NodeRef` () to
the node this anchor tracks for re-anchoring. `node` is entirely absent for
a free-floating anchor (never re-anchored, never detached) — including one
whose original node was hidden/deleted and therefore no longer exists in
this file at all (a writer degrades such an anchor to node-less rather
than emit a dangling reference; its point still round-trips). Unlike a
genuinely free-floating anchor, though, a writer degrading a dead one this
way ALWAYS emits `detached: true` on that entry, whatever the live
document's stored flag happened to be — a node-less anchor that used to
track something and a node-less anchor the user drew free-floating on
purpose must never be indistinguishable on load; the former is never
silently written as a healthy-looking free anchor.

`detached` (`bool`, default `false`, omitted when `false`) marks an
annotation whose anchored node was deleted or consumed (by a boolean,
slice, push-through, or an equivalent operand-consuming op), or whose
captured `curve` couldn't survive a non-similarity transform on its node (a
squashed circle has no valid radius) — kept, not deleted, and rendered in a
warning color rather than silently showing stale geometry. It is
**stored**, not re-derived on load: the kernel decides it at the moment of
the mutation that caused it (an event the file's post-mutation state alone
cannot always reconstruct — a distorted `curve`, in particular, looks like
any other valid circle without this flag) and it round-trips unchanged,
except for the save-time backstop above.

A reader MUST reject (never repair) an annotation with: an unknown `kind`
or `radial_kind`; a non-finite coordinate anywhere (`p`, `offset`, `curve`);
a non-positive `curve.radius`; coincident `a`/`b` anchors on a `"linear"`
entry; or a missing field required by its own `kind` (e.g. `"leader"`
without `text`).

`tags` (v18+, optional, omitted when empty) lists the tag paths the
annotation carries, root-first, as a node's `tags` do (§4.10): an
annotation with a hidden tag, or a tag below a hidden one, is not drawn.
A `.skp` dimension or text takes its layer's tag. A manifest declaring
`format_version < 18` that carries a non-empty `tags` on an annotation, or
any annotation with an empty tag path or path segment, MUST be rejected.

`sid` (v19+, required at v19+) is the annotation's stable id (§4.2), the
same table every other entity draws from: a `.skp` save back uses it to
keep an unchanged SketchUp dimension's own record across saves, and the
Hew API addresses an annotation by it (`ann_<hex>`). Gated both ways
like every `sid`: present in a manifest declaring `format_version < 19`
it is a smuggled field and the file is rejected; missing or duplicated
at v19+ it is a fatal, typed error. A pre-v19 file's annotations are
minted fresh ids in list order on load, as the v14 upgrade does for the
other kinds.

**Version gate.** `annotations` is not merely field-optional: a manifest
declaring `format_version < 13` MUST NOT carry a non-empty `annotations`
array at all. A reader encountering one anyway (a hand-edited or
non-conforming file smuggling the field under a stale declared version)
MUST reject the file with a typed, malformed-manifest error rather than
silently loading it — the same reject-not-repair posture §2 applies to a
too-NEW `consumed` list, mirrored for a field introduced too early instead
of retired too late.

### 4.9 Document tree

`roots` is the ordered list of top-level `NodeRef`s — objects, groups, and
instances with no parent group. Combined with `groups[].members`, recursively
expanding every `"group"` `NodeRef` from `roots` reaches every world-placed
object and instance exactly once. Component-definition members () are
reached only through an instance's `def`, never through `roots` or a group.

### 4.10 Tags

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

### 4.11 Camera

`camera` (manifest v13+, optional) is the camera's working view at the
moment the document was last saved (docs/design/camera.md §5): which
projection was active, its field of view, and where the eye/target/up sat,
in world space.

```jsonc
"camera": {
  "projection": "perspective",   // "perspective" | "parallel"
  "fov_deg": 45.0,                // perspective vertical fov, degrees
  "parallel_height": 5.96,       // v19+: what the view spans at target, meters
  "eye": [4.0, -6.0, 3.0],
  "target": [0.0, 0.0, 0.0],
  "up": [0.0, 0.0, 1.0],
  "image_width": 25.4,           // v24+, optional: Match Photo image width, meters
  "aspect_ratio": 1.5            // v24+, optional: the held width-to-height ratio
}
```

- **`projection`** — `"perspective"` or `"parallel"`; any other value is a
  fatal, typed load error.
- **`fov_deg`** — the perspective vertical field of view in degrees. Present
  (and round-tripped) even when `projection` is `"parallel"`, where it is
  currently meaningless to the ortho frustum itself but is what the camera
  returns to on a later toggle back to perspective — mirroring the app's
  own `CameraRig`, which keeps a single persistent fov across projection
  toggles rather than one per projection.
- **`parallel_height`** (v19+) — the height the view spans at `target`, in
  meters: the parallel frustum's own height under `"parallel"`, which does
  not follow the eye distance (a SketchUp parallel view keeps exactly
  this, with its eye wherever the file left it); under `"perspective"`
  the height that view spans at the target, so a toggle to parallel keeps
  the framing. Always written. A reader of an older file derives it as
  `2 · |eye − target| · tan(fov_deg / 2)`, which is how such a file was
  shown; a value that is not finite and positive, or one present in a
  manifest declaring `format_version < 19`, is a fatal, typed error.
- **`image_width`**, **`aspect_ratio`** (v24+, optional, omitted when
  unset) — a `.skp` camera set to an image (Match Photo) holds the width
  the view spans in meters and the width-to-height ratio the view is held
  to. They are carried and saved, never read by navigation. A `.skp` opens
  with them (a file's zero is unset), and a save back or fresh write
  writes them, zero for unset. Setting the camera to a view that looks the
  way the held one does keeps them; setting it to a view that moved
  replaces them with unset (a save back then also writes the file
  camera's two-point perspective and 2D zoom and pan only while the view
  is the file's own). Each is finite and positive; one in a manifest
  declaring `format_version < 24` is malformed and MUST be rejected.
- **`eye`**, **`target`**, **`up`** — world-space xyz triples: the camera
  position, the point it looks toward, and its up direction. A non-finite
  coordinate (in `fov_deg` or any of the three triples) is a fatal, typed
  load error, never clamped or defaulted.

**Absence is the steady state, not an edge case.** A document that never
explicitly saved a camera view — every file from before this version, and
a v13+ document whose app never called the equivalent of
`Document::set_camera_state` — has no `camera` block at all. A reader MUST
treat its absence as "apply today's home framing" (whatever the app's
default startup view is), never as an error and never by manufacturing a
placeholder view.

**Not undoable.** Unlike every other piece of document state this format
persists, the camera view is deliberately **outside the document's
undo/redo history** — changing your viewpoint is not a model edit, matching
how SketchUp itself treats the camera. An app sets it directly (mirroring
how the tag-visibility registry and per-node user-hidden flags above are
also persisted but not undoable) and typically does so once, right before
a save, so the persisted view reflects "what you were looking at when you
last saved" rather than every transient camera position visited along the
way.

### 4.12 Movable drawing axes

The top-level `axes` object (manifest v13+) is a document-level coordinate
frame that reorients drawing and inference away from the world frame — no
geometry moves when it changes. It carries `origin` and two unit,
mutually-perpendicular directions `x` (red) and `y` (green); the third
("blue") axis is never stored — it is always `x × y` — so a reader
recomputes it on load rather than trusting a stored value that could
disagree with `x`/`y`.

Omitted entirely at world identity (origin `[0,0,0]`, `x`=`[1,0,0]`,
`y`=`[0,1,0]`) — the only state every pre-v13 file is in, so an unmoved
document's manifest is unaffected by this field's existence. A reader MUST
reject, never repair, a present `axes` whose `x`/`y` are not each unit
length or not mutually perpendicular (rule 4), and MUST reject a file
declaring `format_version < 13` that carries an `axes` key at all (a
version-gated field smuggled into an older file — see the field-by-version
table in §2).

Ground-plane rendering (the ground grid) and Scale/Section/standard views
are NOT affected by this frame — they stay world-aligned regardless of the
document's axes.

### 4.13 Section plane

`section_plane` (manifest v16+, optional) is the document's one
non-destructive clipping plane (docs/design/scenes.md §4): `origin` (world
xyz), `normal` (world xyz, **unit length**, pointing at the side the cut
removes), and `active` (whether it is currently cutting; a placed-but-
inactive plane keeps its position). Omitted entirely when no plane is
placed — the only state every pre-v16 file is in.

A reader MUST reject, never renormalize, a `normal` whose length is not
within the section-normal tolerance of 1, or any non-finite coordinate
(rule 4), and MUST reject a file declaring `format_version < 16` that
carries the key at all (a version-gated field smuggled into an older file
— see the field-by-version table in §2).

**Not undoable.** Like `camera` (§4.11), the section plane is view state:
placing, moving, toggling, or deleting it never enters the document's
undo/redo history. It is captured **by value** into Scenes (§4.14) — a
deliberate departure from SketchUp's plane-as-entity model, so a document
needs no plane list to hold "assembled / section A / section B" as three
Scenes on one plane.

### 4.14 Scenes

`scenes` (manifest v16+, optional) is the ordered list of the document's
saved views (docs/design/scenes.md). Array order is tab order. Omitted
entirely when empty. Each entry:

- **`sid`** — the Scene's stable id, minted from the same counter as
  every entity's `sid` (§4.2) so it can never collide with one; a Scene is
  NOT an entity (it carries no `attrs`, is not a `NodeRef` target, and is
  addressed only through this list). A duplicate against another Scene or
  against any entity `sid` is a fatal, typed error. A loader re-seats its
  mint counter past the maximum of entity and Scene sids alike.
- **`name`** — non-empty and **unique** across the document's Scenes (the
  Hew API addresses Scenes by name); a duplicate or empty name is a fatal,
  typed error.
- **`description`** — free text; omitted when empty.
- The five **captured properties**, each optional. **Absence means "not
  captured"**: activating the Scene leaves that aspect of the view alone.
  Presence means the Scene restores it:
  - `camera` — the same shape and validation as the top-level `camera`
    block (§4.11).
  - `hidden_nodes` — sorted list of `sid`s of the objects/groups/instances
    that are user-hidden (§4.10) in this Scene. A **full set**, not a
    delta: any node not listed is visible, so geometry created after the
    Scene was captured is visible in it. May be nested (a member of a
    group, or of a component definition — a hidden definition member is
    hidden in every instance, exactly as the per-node `hidden` flag
    behaves). May be empty (captured: nothing hidden).
  - `hidden_tags` — sorted list of `sid`s of tag-registry entries (§4.10)
    whose `hidden` flag is on in this Scene; the same full-set posture.
  - `section` — either `null` (captured: **no** plane placed) or a
    section-plane object with the §4.13 shape and validation (captured:
    this plane, by value). This is the one place JSON `null` and absence
    mean different things.
  - `display` — `{grid, axes, guides}` booleans, the editor's display
    toggles; opaque to the file format's readers other than the app.
  - `view_style` (v19+) — the look the Scene restores along with its
    display toggles, the §4.15 shape: a `.skp` scene's saved style
    (backdrop and axes) opens here, and a Scene captured in Hew takes the
    document's own `view_style` when it has one. Absent when the Scene
    captures no look (a document shown Hew's way).
  - `transition_s`, `delay_s` — the Scene's own transition time and delay
    in seconds (finite, not negative), absent when the document's own
    applies. Optional on any manifest version; not captured view state.

**References.** Every `sid` in `hidden_nodes` / `hidden_tags` MUST name a
live entity of the file (an object, group, or instance for the former; a
tag-registry entry for the latter). A dangling one is a fatal
`DanglingReference`, never dropped. **Writers prune**: an in-memory Scene
may hold the sid of a deleted node or tag (kept so that undoing the
deletion re-links it), and the writer filters those out at save time —
deterministically, so byte-identical output holds — which is why a
conforming file never carries one.

**Not undoable.** Adding, updating, renaming, reordering, or deleting a
Scene, and activating one, are all outside undo/redo history — the same
posture as `camera`, `axes`' cousins in the view-state family (§4.10,
§4.11, §4.13). Unlike those, an app treats Scene edits as document
changes for its dirty flag (a lost Scene is real work lost). Activation
writes the captured tag `hidden` flags, per-node `hidden` flags, and
`section_plane` back into the document; the camera and display are
handed to the app.

Thumbnails are never stored: they are derived at runtime, and GPU output
is not deterministic (§1).

### 4.15 View style

`view_style` (manifest v19+, optional) is how the document asks to be
shown: its backdrop and whether the model axes are drawn. A SketchUp
model carries a style, and a document opened from a `.skp` keeps the
part of it Hew can honor today; a `.skp` written from a document with a
`view_style` carries it in the file's display settings. Omitted entirely
for a document shown Hew's own way (the theme's backdrop, the ground
grid, the axes) — the only state every pre-v19 file is in.

```jsonc
"view_style": {
  "background": [255, 255, 255],   // sRGB, behind everything
  "sky": [189, 209, 208],          // optional: the sky's color above the horizon; absent = no sky
  "ground": [210, 208, 185],       // optional: the ground plane's color; absent = no ground
  "axes": false,                   // whether the model axes are shown
  "section_cut_color": [0, 0, 0],  // v24+, optional: the cut outline's color
  "section_cut_width": 3,          // v24+, optional: its width in pixels
  "section_fill": false,           // v24+, optional: whether a cut face is filled
  "section_fill_color": [0, 0, 0], // v24+, optional: the fill's color
  "active_section_color": [255, 135, 0], // v24+, optional: the cutting plane's color
  "guides_color": [0, 0, 0],       // v24+, optional: the guides' color
  "guides_hidden": false           // v24+, optional: the style hides guides
}
```

The section and guide settings (v24+, each optional and omitted when
unset) come from a `.skp` style and are written back to one, colors
keeping the file's alpha. The viewport draws the cut outline in
`section_cut_color` at `section_cut_width`; the rest are carried without
being drawn (the fill, the cutting plane's color and the guides' color
and visibility have no Hew rendering yet). Settings with no Hew
counterpart at all (a style's hidden geometry and hidden objects, the
section display bits, a section plane's name and symbol) are not in the
manifest: a save back leaves them as the file has them. A manifest
declaring `format_version < 24` that carries any of these keys MUST be
rejected.

With `sky` absent the background shows above the horizon; with `ground`
absent no ground plane is drawn (Hew's grid lines, a drawing aid the
user toggles, still draw on the ground). Edge and face styles are not
part of this record; they wait for a styles feature of their own.

**Not undoable.** Like `camera` (§4.11), the view style is view state
outside the document's undo/redo history, persisted with the document
and applied on load. A file declaring `format_version < 19` that carries
a `view_style` key is malformed for its own declared version and MUST be
rejected (reject-not-repair, as every version-gated field).

### 4.16 Geo-reference

`geo` (manifest v25+, optional) is the coordinate reference system the
model is placed in: where the model origin sits on a map. It is data a
document carries (a `.skp` keeps it through a save back); it moves no
geometry and nothing reads it as a transform.

```jsonc
"geo": {
  "name": "NAD83 / Oregon North",       // optional text, omitted when empty
  "description": "survey grid",         // optional text
  "easting": 2500000.5,                 // f64, model origin, in the system's own map unit
  "northing": 250000.25,                // f64
  "height": 12.75,                      // f64
  "x_axis": [0.0, 1.0],                 // [f64;2], the model x axis in the map plane (a vector, not an angle)
  "scale": 0.9999,                      // f64, map scale factor
  "geodetic_datum": "NAD83",            // optional text, kept as the file names it
  "vertical_datum": "NAVD88",
  "map_projection": "Lambert Conformal Conic",
  "map_zone": "3601",
  "map_unit": "foot",                   // the unit's name as the file holds it (METRE, foot, US survey foot)
  "map_unit_metres": 0.3048,            // optional f64 > 0, meters per map unit when recorded
  "tgl_id": "EPSG:2913",                // optional text
  "calibration_file_name": "site.cal"   // optional text
}
```

`easting`, `northing`, `x_axis` and `scale` are required; every text key and
`map_unit_metres` is omitted when empty or unrecorded. Names are not
interpreted: Hew keeps the datum, projection, zone and unit as the file
spells them and keeps no registry of them. The angle of `x_axis` from map
east is `atan2(x_axis[1], x_axis[0])`, undefined for a zero vector (no
orientation recorded). A reader MUST reject, never repair, a record with a
non-finite number, a `map_unit_metres` that is not greater than zero, or a
key not listed here, and a manifest declaring `format_version < 25` that
carries `geo` at all.

**Undoable.** Setting or clearing the record is one undo step, restoring
the recorded previous record verbatim; setting the record the document
already holds records nothing. A `.skp` opened into an empty document
brings its record in without history (as it does the camera and the view
style); an import into a document that has work keeps the document's own.
A `.skp` save writes the record in the 2026 container, keeping the file's
own trailing records Hew does not read; the releases 3 to 2020 stream
containers have no such record and omit it.

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
