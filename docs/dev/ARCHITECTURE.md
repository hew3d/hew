# Hew — Architecture (orientation)

Hew pairs SketchUp's interaction model — draw on a face, push/pull,
inference snapping everywhere — with a solids-first data model: every
enclosed volume is a discrete, watertight Object, and Objects never
merge by accident. One pure Rust kernel, compiled to WebAssembly, drives
one TypeScript/React UI shipped two ways: a Tauri desktop app and a
static web app, rendered with three.js on WebGL2.

This page orients a contributor in a few minutes. The full treatment —
every design position, mechanism, and section number that code comments
cite — is [agents/ARCHITECTURE.md](../agents/ARCHITECTURE.md); section
references below (§n) point into it.

## The data model in five entities

A document is a tree of:

- **Objects** — watertight solids, each owning an island of half-edge
  mesh geometry with a tracked watertight/leaky state.
- **Groups** — geometry-free containers; the unit of "these move
  together." Transforms bake into the leaf Objects.
- **Component definitions + instances** — one definition owns geometry;
  each instance places it with an invertible pose (the one place a
  persistent pose exists — everything else is baked).
- **Sketches** — drawn-but-not-extruded profiles, first-class 2D
  entities, never zero-thickness solids (§2.6).
- **Guides** — construction lines and points; snap targets only.

The rules that make it feel like SketchUp without SketchUp's failure
modes:

- **Stickiness is scoped to an Object** (§2.4). Inside one Object,
  edges split faces, closed loops become faces, coincident geometry
  merges. At the Object boundary it all stops — two flush Objects stay
  two Objects.
- **Extrusion manufactures the Object** (§2.2). Pulling a closed
  profile off its plane creates a watertight solid; the profile becomes
  its base face and leaves the sketch.
- **Combination is always explicit** (§2.3). Union, Subtract,
  Intersect, or Group — never proximity. Booleans resolve coplanar
  contact exactly and refuse pure tangency rather than guess.
- **Editing context, not selection, decides where strokes go** (§2.5).
  Entering a Group or Component instance opens a kernel **session
  frame** (§2.11): the contents temporarily become ordinary top-level
  world geometry, so the entire unmodified tool set works inside any
  container. Frames nest, close LIFO, and are never serialized.
- **Push-through is the only way to remove material** (§2.10).
  Deleting a lone face or edge is unsupported by design — that is the
  door back to leaky shells.

## Crate topology

| Crate | Responsibility |
|---|---|
| `kernel` | Mesh, document tree, watertightness, booleans, undo, `.hew` serialization |
| `inference` | Snapping queries over a spatial index |
| `tessellate` | Kernel topology → render buffers |
| `mesh-heal` | Shared foreign-mesh healing (weld, stitch, orient) for every importer |
| `mesh-export` | STL / 3MF / glTF / USDZ writers |
| `dae-import`, `gltf-import`, `skp-import`, `stl-import` | One converter per foreign format, all feeding `mesh-heal` |
| `softrender` | Headless software rasterizer (`hew.view.snapshot`) |
| `hlr` | Hidden-line removal + SVG, for vector printing and export |
| `pdfwrite` | Minimal PDF writer for Save PDF and headless printing |
| `api` | The Hew API registry, commands, and codegen |
| `hew-cli` | Command-line API host: script runner, one-shot dispatch, MCP, `--live` |
| `wasm-api` | The only WASM boundary — everything above, exposed to the UI |
| `app/` | TypeScript/React UI: viewport, tools, panels |
| `shells/tauri`, `shells/web` | Desktop and web shells around the same `app/` |

Three boundaries are non-negotiable (§3.1):

1. `kernel`, `inference`, and `tessellate` never depend on UI, I/O, or
   network crates.
2. Importers depend on the kernel; the kernel never depends on an
   importer. It has no knowledge of any foreign format.
3. Kernel types cross into JavaScript only in `wasm-api`.

The kernel runs as WebAssembly on every platform, desktop included
(§3.2): one identical code path, no IPC geometry serialization, and the
purity rules keep a native-process kernel available as a later escape
hatch without paying for it now.

## Rules that shape the code

- **No silent geometry repair** (§5.3). An operation that would produce
  invalid topology fails with a typed error. The one bounded exception
  is the import boundary, where foreign meshes are healed before the
  kernel ever sees them.
- **Validation before commit** (§5.2). Debug builds validate topology
  after every mutation; release builds clone-mutate-validate on every
  user-reachable operation, so failure leaves the original untouched.
- **Determinism is an invariant** (§5.5). No hash-order-dependent
  collections in kernel crates; the same operations always produce
  byte-identical results. This is what makes session recordings
  reliable reproducers and `.hew` output byte-stable.
- **One epsilon location** (§5.4). All lengths are `f64` meters; every
  tolerance is a named constant. Native geometry is held to
  nanometer-scale coincidence; imported geometry gets a deliberately
  wider planarity gate.
- **Undo never corrupts** (§5.7). Recorded inverses replay guard-exempt
  but proof-carrying: the result must match a recorded geometric
  fingerprint, then is aligned to the recorded bits. Undo can fail
  typed on a kernel bug; it cannot drift or corrupt.
- **The licensing wall** (§4). Nothing derived from the SketchUp SDK —
  headers, constants, or knowledge obtained under its license — may
  enter the dependency chain. `.skp` support exists only through
  [OpenSKP](https://github.com/hew3d/openskp), a clean-room reader.
- **Plugins will run sandboxed, never in-process** (§4). A plugin is a
  separate program reaching Hew through the documented API. This keeps
  the license exception true by construction and a crashed plugin
  harmless.

## Testing, bottom-up

Property tests at the kernel layer carry the load (extrude-is-
watertight, push/pull round-trips, split-merge identity), deterministic
replay exercises recorded sessions headlessly against a known-good
hash, and end-to-end tests stay thin — reserved for flows that only
exist with real UI, rendering, and input in the loop (§5.6). Loading a
`.hew` file runs the same validator a live mutation does.

Start working: [DEVELOPMENT.md](DEVELOPMENT.md) has setup, commands,
and the numbered non-negotiable rules that code comments cite.
