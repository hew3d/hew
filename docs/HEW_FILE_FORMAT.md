# Hew Native File Format (`.hew`)

Status: **v1**, introduced in milestone  (ARCHITECTURE.md).

This document is the **contract** for the native format. The kernel serializer
(`crates/kernel/src/serialize.rs` + `Document::save`/`Document::load`) and all
external converters (`step-io`, `skp2native`) must agree with it byte-for-byte.
Per docs/DEVELOPMENT.md's file-format rule, **this spec is updated in the same commit as
any serialization change**, and a version constant is bumped (below).

## 1. Design goals

- **Open & documented.** No proprietary knowledge; anything here is reproducible
  from this file alone.
- **Deterministic.** Saving the same in-memory document twice yields **identical
  bytes** — golden-file tests and content-addressed storage depend on it.
- **Validating, never repairing.** Loading runs the full topology validator and
  fails with a typed error on any inconsistency (DEVELOPMENT.md rule 4). A tampered or
  truncated file never yields a quietly-broken document.
- **Portable identity.** Generational slotmap handles are *not* stored. Every
  saved entity gets a dense, stable integer id; references use those ids and are
  remapped on load.

## 2. Container

A `.hew` file is a **ZIP archive**, entries stored with the **`stored`
(uncompressed) method** in v1 (texture image bytes are already compressed; a
future version may enable DEFLATE behind the same layout). Entries:

| Path                    | Contents                                                    |
|-------------------------|-------------------------------------------------------------|
| `manifest.json`         | UTF-8 JSON: document tree, materials, sketches, metadata.   |
| `geometry/obj_<id>.bin` | One binary geometry buffer per live object (`<id>` = dense). |
| `textures/tex_<id>.<ext>` | Verbatim authored image bytes, one per textured material (`<ext>` = `png`/`jpg`). |

For determinism, entries are written in a fixed order: `manifest.json` first,
then `geometry/obj_*.bin` in ascending id, then `textures/tex_*` in ascending id.
ZIP timestamps/external-attributes are written as fixed constants (0), never the
wall clock.

## 3. Identity model

On **save**, each live entity is assigned a dense `u32` id by iterating its
slotmap in key order (ascending slot — deterministic). Separate id spaces per
kind: objects, groups, components, instances, sketches, materials, guides, and
— *within a sketch* — sketch-vertices, sketch-edges, sketch-regions.

**Not persisted** (a save is the live, visible state, like SketchUp):
- the undo/redo logs (after load the undo stack is empty);
- `hidden` records (objects/groups/instances/components whose creation was undone) —
  they and anything referencing them are skipped.

On **load**, entities are inserted into fresh slotmaps and old-dense → new-key
maps resolve every reference. A reference to an id that is absent → typed
`LoadError` (never silently dropped).

`Option<id>` references encode as: JSON `null`/omitted = `None`; in binary, the
sentinel `0xFFFF_FFFF` = `None`.

## 4. `manifest.json`

```jsonc
{
  "format_version": 4,            // — bumped on any manifest-shape change
  "geometry_version": 2,          // GEOMETRY_FORMAT_VERSION of the .bin buffers
  "app": "hew",
  "app_version": "0.1.0",

  "materials": [
    { "id": 0, "name": "Red", "color": [220, 50, 40, 255],
      "texture": {                // omitted when the material is flat color
        "asset": "textures/tex_0.png",
        "format": "png",          // "png" | "jpg"
        "world_size": [1.0, 1.0]  // meters per image tile [w, h], both > 0
      } }
  ],

  // ALL live objects (both world objects and component-definition members).
  // Geometry lives in the referenced buffer; base_material is the object
  // default material (null/omitted = None). `name` (v2+) is an optional display
  // name (e.g. carried in from an import); omitted = unnamed → positional label.
  // `tags` (v3+) is a list of root-first tag-path segments (e.g. SketchUp Layers
  // hierarchy); omitted / empty = no tags.
  "objects": [
    { "id": 0, "geometry": "geometry/obj_0.bin", "base_material": 0,
      "name": "Counter_Base",
      "tags": [["Architecture", "Walls"], ["Level", "L1"]] }
  ],

  // Merge groups: membership only, ordered. Nesting allowed.
  // `name` (v2+) optional, as for objects.
  // `tags` (v3+) list of root-first tag-path segments; omitted / empty = no tags.
  "groups": [
    { "id": 0, "members": [ {"kind":"object","id":0}, {"kind":"group","id":1} ],
      "name": "MyGroup",
      "tags": [["Architecture"]] }
  ],

  // Component definitions: a flat, ordered set of definition-local objects.
  // `name` (v2+) is the definition's display name; an instance with no own name
  // shows its def's name. Components carry no tags (tags live on instances).
  "components": [
    { "id": 0, "members": [2, 3], "name": "MyComponent" }   // object ids that are this def's members
  ],

  // Component instances: a def placed at an invertible pose.
  // `name` (v2+) optional per-instance display name; omitted = use the def name.
  // `tags` (v3+) list of root-first tag-path segments; omitted / empty = no tags.
  "instances": [
    { "id": 0, "def": 0,
      // row-major 3x4 affine (def-local -> world); = Transform::to_affine().
      // May be reflective / non-uniform (poses are not baked).
      "pose": [1,0,0,0, 0,1,0,0, 0,0,1,0],
      "tags": [["Furniture"]] }
  ],

  // First-class 2D sketches. Sub-ids are dense within the sketch.
  "sketches": [
    { "id": 0,
      "plane": [nx, ny, nz, offset],            // unit normal + plane offset
      "vertices": [ {"id":0, "p":[x,y,z]} ],
      "edges":    [ {"id":0, "from":0, "to":1} ],
      "regions":  [ {"id":0, "outer":[0,1,2,3], "holes":[[4,5,6]]} ]
    }
  ],

  // Construction guides (format_version 4+): non-solid alignment helpers,
  // never rendered as geometry, never affecting watertightness. `kind` is
  // "line" | "point". A line's `p` is its origin and `dir` its unit direction
  // (omitted for points). Missing/omitted entirely (v1-v3 files) = no guides.
  "guides": [
    { "id": 0, "kind": "line", "p": [0.0, 0.0, 0.0], "dir": [1.0, 0.0, 0.0] },
    { "id": 1, "kind": "point", "p": [2.0, 3.0, 0.0] }
  ],

  // The document's top-level nodes, in order (NodeRef into objects/groups/instances).
  "roots": [ {"kind":"object","id":0}, {"kind":"instance","id":0} ],

  // (sketch_id, region_id) pairs already extruded into a solid (no longer
  // offered for extrusion). Both ids are the dense ids above. Sorted ascending.
  "consumed": [ [0, 0] ]
}
```

`NodeRef.kind` ∈ `"object" | "group" | "instance"`. Each object id appears in
exactly one structural position: a definition member (in `components[].members`)
or somewhere in the world tree (a `roots`/`groups` `NodeRef`). The plane is the
kernel `Plane` as `[normal.x, normal.y, normal.z, offset]`; rebuilt on load via a
normal+offset constructor (planarity re-checked against `tol::PLANE_DIST`).

## 5. Geometry buffer (`geometry/obj_<id>.bin`)

Little-endian throughout. Encodes an object's **generators**, not its half-edge
graph; the loader rebuilds topology via `Object::from_faces_with_holes`
(`crates/kernel/src/build.rs`) and then runs the validator.

Material ids in the buffer are the same **dense** ids as the manifest (one id
space), so the codec is parameterized by the document's material mapping:

```rust
// 0xFFFF_FFFF is reserved for None; for a material-free object the closures
// are never invoked.
pub fn encode(&self, material_dense: &impl Fn(MaterialId) -> u32) -> Vec<u8>;
pub fn decode(bytes: &[u8], dense_material: &impl Fn(u32) -> Option<MaterialId>)
    -> Result<Object, DecodeError>;
```

`Document::save` builds the `MaterialId -> u32` map once and passes it to every
object; `Document::load` inserts the palette first, then passes the reverse.

```
offset  type        field
0       [u8; 4]     magic = b"HEWG"
4       u32         version = GEOMETRY_FORMAT_VERSION (3)
8       u8          watertight: 0 = Open (leaky), 1 = Watertight
9       u32         base_material id (0xFFFFFFFF = None)   // mirrors manifest objects[].base_material
13      u8          imported flag (v3+): 0 = strict native, 1 = imported   // omitted in v1/v2 → 0
        u32         vertex_count
        f64 * 3 * vertex_count   vertex positions, in vertex-slot order (x, y, z)
        u32         face_count
        — per face, in face-slot order:
            u32       material id (0xFFFFFFFF = None)
            u8        uv_frame flag (v2+): 0 = none, 1 = present   // omitted in v1 buffers → None
            f64 * 8   uv_frame, only when flag = 1: s.x s.y s.z  t.x t.y t.z  u0 v0
                      (oriented planar UV: uv = (s·p + u0, t·p + v0);  extension, imported texcoords)
            u32       outer_count
            u32 * outer_count    outer-loop vertex indices (0-based into positions, in loop order)
            u32       hole_count
            — per hole:
                u32       hole_vertex_count
                u32 * hole_vertex_count   hole-loop vertex indices
```

Notes:
- **Face planes are not stored.** On load, each face plane is recomputed from its
  outer-loop vertices via `Plane::from_polygon` — the exact construction the
  object was first built with. Baked object transforms are always orientation-
  preserving (det > 0;  refuses reflections), so winding — and thus the normal
  orientation `from_polygon` derives — round-trips faithfully. (Mirrored geometry
  only ever exists as a non-baked instance *pose*, stored in the manifest.)
- `watertight` is stored **and** cross-checked: the value the rebuilt topology
  computes must match the stored byte, else `Corrupt` (cheap tamper detection).
- Index/count validation: any out-of-range index, a count that overruns the
  buffer, or a face with `outer_count < 3` → `Corrupt { offset, what }`. A buffer
  that ends early → `Truncated`.
- **`uv_frame` (geometry v2)** carries an importer-fit oriented planar UV mapping
  per face; faces without one (everything drawn in Hew) store flag `0` and render
  via the  `world_size` planar projection. Decode is version-gated: a **v1**
  buffer has no flag byte, so every face loads with `uv_frame = None` — older
  `.hew` files round-trip unchanged.
- **`imported` flag (geometry v3)** selects the object's
  face-planarity invariant tolerance on load. `0` (native, the default) validates
  faces at the strict `tol::PLANE_DIST` (1e-9 m); `1` validates at the wider
  `tol::IMPORT_PLANE_DIST` (1e-3 m) so faces from f32-quantized imports
  (SketchUp/COLLADA), flat only to ~0.1 mm, are accepted as the planar polygons
  they represent instead of being rejected. Decode is version-gated: a **v1/v2**
  buffer has no flag byte → strict. The plane is still recomputed (planes aren't
  stored), so the gate is the only thing that differs.

## 6. Versioning

Two independent `u32` versions:
- `format_version` (manifest/container shape) — `MANIFEST_FORMAT_VERSION`.
- `geometry_version` (the `.bin` layout) — `GEOMETRY_FORMAT_VERSION`.

Bump the relevant constant on any layout change, extend this spec **in the same
commit**, and regenerate the golden files (). A loader accepts any version it
understands — `0 < found ≤ MANIFEST_FORMAT_VERSION` — and rejects newer ones with
`UnsupportedVersion { found }` rather than guessing. **Older** manifests still
load: every field added in a later version is `#[serde(default)]`, so missing
fields default (`format_version` 1 has no node `name`s → all `None`).

`format_version` history:
- **v1** — initial manifest shape.
- **v2** — optional `name` on object/group/component/instance entries.
- **v3** — optional `tags: Vec<Vec<String>>` (root-first tag-path segments) on
  object/group/instance entries. Components carry no tags (tags sit on instances).
  Missing `tags` fields in older files default to `[]` via `#[serde(default)]`.
- **v4** — optional `guides` array — construction lines + points; missing
  ⇒ none.

## 7. Conformance tests

Living in `crates/kernel/tests/`:
- **Roundtrip**: `load(save(doc))` is structurally + geometrically equal to `doc`
  (proptest over generated documents).
- **Determinism**: `save(doc) == save(doc)`, byte-for-byte.
- **Golden files**: committed `.hew` fixtures (a holed solid, a merge group, an
  instanced component incl. a mirrored pose, painted + textured + base-material
  objects, a sketch with a consumed region, a construction guide line + point)
  that must save bit-identically and re-load. Regenerated only on an
  intentional version bump.
- **Negative**: truncated / garbage / tampered buffers and dangling manifest
  references → the typed errors above, never a panic, never silent repair.
