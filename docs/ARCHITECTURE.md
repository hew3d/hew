# Hew — Architecture

Hew is a cross-platform 3D modeler. This document describes the shape of the
system: the data model, the crate and module topology, the design positions
that shaped it, and the mechanisms that keep it robust. It is meant to orient
a reader who is about to work in the codebase, not to justify every choice —
the commit history and the code do that.

## 1. Overview

Hew combines two things that don't usually travel together: an interaction
model borrowed from SketchUp, and a data model borrowed from solid CAD.

The interaction model is direct modeling. Draw a closed shape on a surface,
push or pull a face to give it depth, and let a pervasive inference engine
snap the cursor to endpoints, midpoints, edges, faces, and axes as you work.
There is no history tree, no parametric feature list, no sketch-then-pad
dialog. What you see is what you have.

The data model is solids-first. Every enclosed volume in a Hew document is a
discrete Object — a watertight solid, tracked as such, that never merges with
another Object by accident. Geometry inside an Object behaves the way
SketchUp geometry behaves (edges split faces, coplanar loops become faces,
push/pull reshapes a face), but that stickiness stops at the Object's
boundary. Two Objects sitting flush against each other stay two Objects until
something explicitly joins them.

This pairing exists to fix the two failure modes that make raw SketchUp-style
modeling frustrating at scale:

- **Accidental welding.** In a pure "sticky geometry" model, ungrouped
  geometry fuses with anything it touches, and un-fusing it later is not
  really possible. Hew scopes stickiness to an explicit container (the
  Object) so touching geometry never welds unless a user asks for that.
- **Hollow non-solids.** A face-soup model has no concept of "solid" beyond
  visual closure — delete one face and a shape that looked whole is now an
  open shell, silently. Hew tracks watertightness as a real, queryable
  property of every Object and refuses operations that would open one
  without saying so.

Architecturally, this is implemented as a single Rust geometry kernel — pure,
UI-free, compiled to WebAssembly — driving a TypeScript/React interface built
once and shipped both as a desktop application (via Tauri) and as a static
web app, rendered with three.js on a WebGL2 baseline. The kernel is the
source of truth for everything that must be exact: topology, watertightness,
booleans, undo, and file I/O. The UI owns everything that is a matter of
taste or platform convention: camera behavior, tool state machines, panel
layout, and session state such as which Object is currently being edited.

## 2. The Data Model

This is the part of Hew that most determines how it feels to use, and it is
the part most worth understanding before touching the kernel.

### 2.1 Document and Object

A Hew document is a tree. Its nodes are:

- **Objects** — discrete, watertight solids. Each Object owns an island of
  half-edge mesh geometry: vertices, edges, and planar faces (including faces
  with holes), with a tracked watertight/leaky state.
- **Groups** — non-destructive structural containers with no geometry of
  their own; a unit of selection and transform over their children.
- **Component definitions and instances** — a definition owns a flat set of
  geometry in its own local coordinate space; an instance places that
  geometry in the tree at an invertible pose. Multiple instances share one
  definition's geometry.
- **Sketches** — first-class 2D drafting entities, not solids (see 2.6).
- **Guides** — construction lines and points, not solids (see 2.9).

Cross-node operations — union, subtract, merge, slice — are always explicit,
document-level commands. Nothing in this tree merges as a side effect of
proximity.

### 2.2 Extrusion automatically creates an Object

Extruding a closed 2D profile is the one operation that manufactures a new
Object without a separate "make solid" step. A drawn rectangle or polygon,
once pulled off its plane, becomes a self-contained watertight solid with no
further action required. This is the concrete expression of "solids by
default": a user never has to remember to group their geometry to keep it
from fusing with the rest of the scene, because the geometry that would fuse
doesn't exist yet — only its 2D sketch outline does, and that outline is
consumed once the extrusion happens: it becomes the solid's base face and
leaves the sketch (see 2.6).

### 2.3 Combination is always explicit

Two Objects only become one via a deliberate command: boolean union,
subtract, or a non-destructive Group. There is no implicit welding across
Object boundaries, no matter how precisely two Objects' faces coincide.
Booleans are implemented as a polygon arrangement operation that reuses the
same face-splitting machinery as ordinary editing, rather than a
triangle-soup CSG library — this lets coplanar contact (shared walls,
stacked solids, coincident faces) be resolved exactly instead of falling back
to numerically fragile epsilon comparisons. A boolean that would depend on
resolving a pure tangency (an edge lying exactly on a face, with no volume
overlap) refuses cleanly rather than guess.

### 2.4 Sticky geometry, scoped to an Object

Inside a single Object, Hew reproduces SketchUp's "sticky geometry" feel:

- Drawing an edge across a face splits that face.
- A closed loop of coplanar edges automatically becomes a face.
- Push/pull reshapes any face region, generating or merging the side walls
  needed to keep the result watertight.
- Coincident vertices and edges merge — but only with other geometry inside
  the same Object.

Inference snapping itself is not scoped this way: the cursor can snap to
anything visible in the whole scene (another Object's edge, a guide, an
axis), because snapping is a read-only query, not a mutation. Only the
merging of geometry into a single mesh is confined to the active Object.

### 2.5 Editing context, not selection, determines stickiness

Which Object a new stroke joins is determined by an editing context — the
Object (or nested path of Group/Object) a user has entered, double-click
style, the same gesture SketchUp uses. Drawing at the top level of the
document creates or extends a Sketch; drawing after entering an Object
applies that Object's sticky rules. Selection was considered as the
triggering signal and rejected: it is a weaker, more modal proxy for "what am
I editing" than an explicit enter/exit context, and it doesn't generalize
cleanly to nested Groups and Components. Editing context lives entirely in
the application layer — it is session state, not part of the persisted
document model.

### 2.6 Sketches are first-class, not zero-thickness solids

A shape drawn but not yet extruded — a rectangle, a polygon, an arc — is a
persistent Sketch entity in the document tree: movable, copyable, and
deletable before it becomes a solid. It is deliberately not modeled as a
degenerate ("zero-thickness") solid Object, for two reasons:

- It would permanently violate the watertightness contract every other
  Object honors, forcing a special case that suppresses the "leaky" state
  for an entire category of entity that was never meant to be watertight.
- Booleans and push/pull are defined over watertight operands. Treating flat
  shapes as solids would force every operation to handle a face-versus-solid
  case in addition to solid-versus-solid, for no benefit.

A Sketch is the larval form of a solid: extruding a region makes the
drawn profile BECOME the solid's base face, so the geometry that bounded
it leaves the sketch — really deleted, not hidden
(docs/design/sketch-solid-model.md). Exactly the edges no surviving
region needs go: an edge shared with a neighboring region stays so the
neighbor remains closed, open chains stay, and an extrusion that empties
a sketch removes the sketch entity itself. Nothing invisible persists, so
nothing can resurrect by side effect — deleting a solid deletes a solid,
and the one road back to an outline is undo, which restores the sketch
and removes the solid in the same atomic step. "What you see is what you
have" is the invariant this buys: every entity in the document is either
visible geometry or nothing.

Re-extruding occupied ground is refused by a gate *derived live* from the
scene, never stored: a region refuses to extrude iff its material
overlaps a coplanar face of a visible solid on the sketch's plane (shared
boundary alone is not overlap, so adjacent construction stays free).
Because the claim is the solid's own face, it is kinematic and global by
construction — it moves when the solid moves, dies when the solid dies or
hides, applies to copies and component instances (through their poses)
exactly as to originals, and holds across every sketch on the plane, so
no fresh sketch, split, merge, or redraw can launder a standing solid's
base into extrudability. Boolean, slice, and push-through results claim
precisely the area their actual geometry stands on, because there is no
inherited bookkeeping to diverge from reality. Files store none of this:
the `.hew` format carries no claim data at v11, and the stored claims of
older versions are ignored on load (HEW_FILE_FORMAT.md).

Within a Sketch, the user-facing units are derived, identity-stable
sub-entities, mirroring how regions already work: **islands** (connected
components — each shape drawn apart from the others selects, deletes, and
moves independently, recomputed on every edit with stable ids) and
**curves** (the facet chain one arc or circle gesture committed, carried as
edge metadata and persisted in the file format, so a drawn curve selects
and deletes as a unit). A curve chain also carries the analytic circle it
was drawn from (center + radius), and extrusion stamps each side wall with
the cylinder it is a chord facet of — durable metadata over the faceted
carrier, propagated under a strict map-or-drop contract and held honest by
the validator. Push/pull on a stamped wall facet acts on the *logical
wall*: an exact radial offset of every facet claiming that cylinder,
derived from the stored axis and radius rather than from any facet's
plane, refusing typed where a neighbor would bend. This is the foundation
of the true-curves plan; the architectural decision and staging live in
`docs/design/true-curves.md`.

### 2.7 Groups and Components are the two ways to compose

Groups and Components answer different questions and stay separate concepts:

- A **Group** answers "which pieces move together?" It has no geometry, no
  pose, and welds nothing; transforming a group bakes the transform into
  every leaf Object beneath it.
- A **Component** (definition + instance) answers "where do copies of this
  thing appear?" A definition owns geometry once; every instance carries its
  own invertible affine pose, which — unlike a baked transform — may include
  mirroring or non-uniform scale, since a pose is applied at render and
  tessellation time rather than baked into stored geometry.

A move, rotate, or scale of a plain Object or a Group bakes the transform
directly into vertex and plane data. This keeps the kernel's core geometry
representation pose-free everywhere except Component instances, which is the
one place a persistent pose earns its complexity.

### 2.8 Materials and Tags

Materials are a document-level palette; each face carries an optional
material reference, and each Object carries an optional default material
that unmarked faces fall back to. A material is a name, a color, and an
optional texture. Textures are stored as opaque, already-encoded image bytes
— the kernel never decodes an image, keeping image codecs out of a crate
that must stay dependency-light and pure — mapped onto a face by an affine
UV frame precise enough to reproduce an imported model's original texture
scale and orientation, not just a generic planar projection.

Tags (Hew's analog of SketchUp Layers) are first-class document metadata used
to group and show/hide Objects, Groups, and Instances independently of the
tree structure they otherwise live in.

### 2.9 Guides

Guides are persisted construction geometry — offset lines, angular lines,
and points — distinct from both Sketches and solid Objects. They carry no
fill and no watertightness, never extrude, and exist purely as durable
reference geometry and inference snap targets, created by measuring and
angle tools and cleared explicitly.

### 2.10 Operations that preserve watertightness by construction

A handful of higher-level operations are worth calling out because each one
is designed so that watertightness can never be broken as a side effect:

- **Push-through.** Pushing or pulling a face past the Object's opposite
  face performs a boolean subtraction of the swept volume rather than
  producing an open shell — this is the *only* way material is removed from
  a solid in Hew.
- **Slice.** Cutting an Object with a plane produces two independent,
  fully watertight Objects sharing a coincident cut face — a real geometric
  split (in the spirit of a CAD "split body" operation), not a virtual
  overlay bolted onto one solid.
- **Delete.** Deletion removes whole nodes (an Object, Group, Instance, or
  guide) at a time. Deleting a single face or edge is deliberately
  unsupported: allowing it would let a watertight solid become a leaky shell
  through an everyday action, reintroducing the exact failure mode the data
  model exists to prevent. Removing material or punching an opening always
  routes through push-through instead.

## 3. Crate and Module Topology

| Crate / module | Responsibility | Depends on |
|---|---|---|
| `crates/kernel` | Half-edge mesh, Document tree (Sketch / Object / Group / Component / Guide), watertightness tracking, booleans, undo, native-format serialization | nothing UI-, I/O-, or network-facing |
| `crates/inference` | Snapping/inference engine: endpoint, midpoint, on-edge, on-face, axis, parallel, perpendicular queries over a spatial index | `kernel` |
| `crates/tessellate` | Kernel topology → render buffers | `kernel` |
| `crates/mesh-heal` | Shared foreign-mesh healing pipeline (weld, dedup, T-junction stitching, orientation, coplanar merge) used by every importer | `kernel` |
| `crates/dae-import` | COLLADA (`.dae`) → Objects/Instances | `kernel`, `mesh-heal` |
| `crates/gltf-import` | glTF/GLB → Objects/Instances | `kernel`, `mesh-heal` |
| `crates/skp-import` | Clean-room SketchUp (`.skp`) → Objects/Instances, built on the `openskp` reader | `kernel`, `mesh-heal`, `openskp` |
| `crates/wasm-api` | `wasm-bindgen` surface exposing the kernel (plus inference and tessellate) to the UI | `kernel`, `inference`, `tessellate` |
| `app/` | TypeScript/React UI: viewport, tools, panels | `wasm-api` (via the compiled WASM package) |
| `shells/tauri` | Desktop shell | `app/` |
| `shells/web` | Static web build | `app/` |

### 3.1 Purity boundaries

`kernel`, `inference`, and `tessellate` never depend on UI, rendering,
filesystem, or network crates, and the dependency arrow between the kernel
and its importers only ever points one way: importers depend on the kernel
to build Objects, the kernel never depends on an importer. The WASM boundary
— the only place kernel types cross into JavaScript — lives exclusively in
`wasm-api`. This means every geometric operation can be fuzzed, proptested,
and reasoned about as ordinary Rust, with no UI or platform state entangled
in it, and it means the kernel is trivially reusable from any host that can
run WebAssembly or link a native Rust crate.

Every public kernel mutation runs a topology validator in debug builds
(manifold edges, closed face loops, consistent watertightness flags, no
orphan half-edges), and geometric operations carry property-based tests for
invariants such as: extruding a closed profile is always watertight;
push/pull followed by its exact inverse restores the original topology;
splitting a face and re-merging the split is the identity operation.

### 3.2 Why the kernel runs as WebAssembly everywhere

The kernel compiles to WebAssembly via `wasm-bindgen` and runs inside the
webview on every platform — including the desktop build, where a
Tauri application could instead have linked the kernel natively into its
Rust host process. Running one identical code path everywhere gives a
synchronous, in-memory API with no serialization of geometry across an IPC
boundary, and it means desktop and web builds can share every bug fix and
every behavior without a native/WASM divergence to keep in sync.
WebAssembly runs close enough to native for this kind of geometry work,
and its 32-bit address space is more than sufficient for the model sizes
Hew targets. Kernel purity (3.1) is what keeps a future escape
hatch — a native-process kernel talking to the UI over a binary IPC channel
for very large models — available without having to pay for it today: a
kernel with no UI or I/O dependencies can be relinked into a different host
without being rewritten.

### 3.3 Rendering

`tessellate` converts kernel topology into flat vertex/index/normal buffers;
the UI hands those to three.js, rendered on a WebGL2 baseline. Faces
carrying an analytic surface reference shade with true per-vertex surface
normals, and the facet seams interior to one curved wall are emitted as a
separate soft-edge buffer the viewport suppresses — a drawn cylinder reads
as one smooth wall while its cap rims stay crisp
(`docs/design/true-curves.md`). WebGL2 is the
lowest common denominator reliably available across the three
platform webviews Hew targets (WebView2 on Windows, WKWebView on macOS,
WebKitGTK on Linux), and a SketchUp-class viewport — flat-shaded faces plus
edge overlays — does not need more than that. WebGPU support in those same
webviews is uneven enough that it is treated as a later, strictly optional
enhancement, not a baseline requirement; `tessellate` exists in part to keep
the kernel decoupled from whichever rendering API is in use, so that
enhancement doesn't touch kernel code.

### 3.4 Interop

Interop with formats outside Hew's own format is handled by standalone
converters that sit at arm's length from the core kernel, rather than by
teaching the kernel about any external format:

- **`dae-import`** reads COLLADA 1.4 (`.dae`), the open interchange format
  SketchUp exports natively, then runs it through the shared healing
  pipeline in `mesh-heal` (vertex welding, degenerate-face dropping,
  duplicate-face removal, T-junction stitching, orientation repair,
  coplanar-triangle merge, and hole reconstruction) to reconstruct editable,
  correctly-oriented, watertight-or-honestly-leaky Objects rather than raw
  triangle soup.
- **`skp-import`** reads SketchUp's native `.skp` format directly, built on
  [OpenSKP](https://github.com/hew3d/openskp), a clean-room reader
  maintained as its own project and consumed as an ordinary versioned
  dependency. It shares the same `mesh-heal` pipeline as every other
  importer.
- **`gltf-import`** reads glTF/GLB through the same importer shape and the
  same shared healing pipeline.
- STEP/IGES interoperability (planned — see `docs/ROADMAP.md`) is designed
  as a standalone converter wrapping OpenCASCADE, invoked as an external
  process rather than linked into the kernel — keeping a large C++
  dependency and its licensing entirely out of the Rust workspace.
- Export to interchange formats is one-way and needs no healing machinery
  (it never reconstructs editable kernel topology from someone else's
  mesh). glTF/GLB reads the already-tessellated, already-rendered scene
  data. STL is sourced from the kernel's **export tessellation** instead:
  each object re-facets its stamped cylinder walls from their analytic
  surface references at a user-chosen resolution — true curves for STL,
  manifold at any setting — falling back to stored facets for any wall
  whose boundary is no longer fully analytic
  (`docs/design/true-curves.md`).

Every importer depends on the kernel and never the reverse, so the kernel's
public surface has no knowledge of COLLADA, glTF, or `.skp` at all — it only
ever sees the Objects, Instances, and Groups an importer constructs from
them.

### 3.5 Native file format

Hew's own format (`.hew`) is a ZIP container holding a JSON manifest (the
document tree, materials, and metadata) plus binary geometry buffers, one
per Object. It is deterministic — saving the same in-memory document twice
produces byte-identical output — and validating rather than repairing:
loading a file runs the same topology validator a live mutation would, and a
truncated or tampered file fails with a typed error rather than loading into
a silently broken document. The format is documented independently of any
one implementation, versioned, and is the single contract every converter
and the kernel's own serializer must agree with.

## 4. Key Design Positions

These are the positions that still shape the codebase today, stated as
reasons rather than as a history of how they were reached.

**A standalone application, not a plugin for an existing CAD host.**
SketchUp-style direct modeling — no history tree, sticky geometry, immediate
push/pull — conflicts structurally with parametric, history-based kernels
built around a recompute model. Hosting Hew's interaction model inside one
of those systems would mean fighting its document model at every turn rather
than building on it.

**A Rust core.** A half-edge mesh kernel is fundamentally pointer-graph
manipulation, and Rust's ownership model turns the failure mode of a broken
mutation from silent memory corruption into a compile error, which matters a
great deal in a domain (geometry) where a corrupted invariant can silently
propagate for a long time before it's noticed. A single toolchain
(`cargo build`/`test`/`clippy`/`fmt`) is also a complete verification loop
with no separate build-system layer to maintain.

**One kernel, two shells.** A single Rust kernel and a single
TypeScript/React UI ship as both a desktop application (Tauri) and a static
web application. Tauri was chosen over Electron for a materially smaller,
better desktop citizen, at the accepted cost of variance across
platform webviews (see 3.3) rather than shipping a bundled browser engine.

**The kernel runs as WebAssembly on every platform, not just the web
build.** Covered in depth in 3.2: one code path, no IPC serialization of
geometry, and a preserved (not prematurely built) escape hatch to a native
kernel process for very large models.

**three.js on a WebGL2 baseline.** The only 3D rendering API reliably
present across every platform webview Hew targets; sufficient for the kind
of viewport a SketchUp-like tool needs; isolated behind `tessellate` so a
later WebGPU path doesn't require kernel changes.

**Interop lives in standalone converters, never in the kernel.** Every
external format — COLLADA, glTF, `.skp`, STEP — is handled by a crate that
depends on the kernel, never the other way around, and that owns all
format-specific parsing and healing logic. This keeps the kernel's surface,
its dependency list, and its test matrix free of any one interchange
format's quirks.

**An open, documented native file format.** The `.hew` container (ZIP +
JSON manifest + binary geometry) is specified independently of any one
implementation and versioned from the start, so it can act as the single
contract between the application and every converter, rather than an
implementation detail reverse-engineered later.

**No dangling edges inside a solid.** A half-edge solid cannot represent a
half-finished cut without breaking its own manifold invariants, so the
kernel only ever accepts complete, boundary-to-boundary cuts. A partially
drawn stroke is tool-layer state in the UI, buffered until it closes; every
state the kernel actually commits is a valid solid.

**Sketches are a first-class entity, never a zero-thickness solid.**
Covered in 2.6. This keeps the watertightness contract meaningful for every
entity that claims to be a solid, and keeps boolean and push/pull operations
from needing a face-versus-solid case alongside solid-versus-solid.

**Stickiness follows editing context, not selection.** Covered in 2.5.
Context is an explicit enter/exit model, generalized to a path through
nested Groups and Components, and lives entirely as application/session
state rather than as part of the persisted document.

**Booleans resolve coplanar contact exactly, and refuse rather than guess
at pure tangency.** Shared walls, stacked solids, and coincident faces are
common in practice, not edge cases, so the boolean engine classifies
coplanar sub-faces by an exact, tolerance-free coverage test instead of
falling back to a numerically fragile epsilon comparison. A genuinely
ambiguous case (an edge lying exactly on a face with no area overlap) is
refused outright rather than resolved arbitrarily. Results are also cleaned
up: coplanar seams a boolean (or a through-cut push/pull) introduces are
dissolved, so two joined tops read as one face — while coplanar edges the
operands already carried (face imprints drawn but not yet extruded) are
preserved, and differing face materials are a hard stop.

**A baked transform for plain geometry; a real pose only for Component
instances.** Move, rotate, and scale write directly into an Object's or
Group's vertex and plane data, keeping the kernel's core representation
pose-free everywhere except the one place — instanced, shared geometry —
where a persistent, invertible pose (including reflection and non-uniform
scale) is the feature, not a complication.

**Groups and Components solve two different problems and stay two
different entities.** A Group is a non-destructive, geometry-free structural
container; a Component definition/instance pair is how geometry gets shared
and instanced. Collapsing the two would make either "share geometry" or
"group without merging" a second-class case of the other.

**Material and texture data are opaque to the kernel.** Textures are stored
as already-encoded image bytes; the kernel never links an image codec. An
affine per-face UV frame is precise enough to reproduce an imported model's
authored texture mapping without needing per-vertex UV storage, which a
flat-faced, solids-first model doesn't otherwise require.

**Push-through is the only way to remove material from a solid.** Covered
in 2.10. Centralizing material removal in one boolean-backed operation is
what makes the "delete a whole node only" restriction on plain deletion safe
rather than merely convenient.

**Units are a display concern layered losslessly over an f64-meter
kernel.** The kernel always works in meters; metric and imperial display
formats (including architectural feet-inches-fractions) are defined as exact
conversions of that one internal representation, so switching units is
never lossy and never touches stored geometry.

**A licensing wall around proprietary format knowledge, enforced as an
admissibility test rather than a blanket ban on a format.** Nothing derived
from a proprietary SDK — its headers, its constants, or format knowledge
obtained under its license — may enter the kernel's dependency chain,
including transitively through an interop crate. What that permits is not
"never support `.skp`," but "support it only through a reader whose entire
provenance is clean" — which is what makes a clean-room reader like OpenSKP,
maintained independently and consumed as an ordinary dependency, an
acceptable way to read a proprietary format without ever touching its SDK.

**Determinism is treated as a kernel invariant, not an incidental
property.** Kernel crates avoid hash-order-dependent collections in favor of
ordered ones, so that the exact same sequence of operations always produces
bit-identical output. This is what makes recording a sequence of operations
and replaying it later a reliable way to reproduce and permanently regress-
test a bug, instead of an approach that only works "most of the time."

## 5. Robustness Model

Robust behavior under floating-point geometry is treated as the project's
central engineering risk, and the mechanisms below exist specifically to
manage it.

### 5.1 Watertightness is a tracked, visible property

Every Object carries an explicit watertight/leaky state as part of its data,
not something inferred by inspection when needed. Operations that could
compromise it are the ones a user has to reach for deliberately (union,
subtract, slice, push-through); an Object that ends up leaky (most often
from import, where the source geometry may genuinely have gaps) is flagged
as such rather than presented as solid.

### 5.2 The topology validator

A validator checks manifold edges, closed face loops, consistent
watertightness flags, and the absence of orphaned half-edges. It runs after
every mutation in debug builds. Because a release WebAssembly build compiles
out debug-only checks, user-reachable mutating operations additionally carry
an always-on validation pass: geometry is cloned, mutated, and validated
before the mutation is allowed to commit, so a failure leaves the original
Object untouched and surfaces as a typed error rather than a crash or, worse,
a silently corrupted document.

### 5.3 No silent geometry repair

If an operation would produce invalid topology, it fails loudly with a typed
error. The kernel never "fixes up" geometry a user's action would have
broken. The one place repair genuinely happens is at an import boundary,
where foreign mesh data is normalized — welded, de-duplicated, re-oriented,
and stitched — before it is ever handed to the kernel's own construction
path; that healing is explicitly bounded to the point where external data
enters the system, and native kernel operations are never subject to it.

### 5.4 Tolerance policy

All kernel lengths are `f64` meters. Every tolerance is a named constant —
there is exactly one place in the kernel where an epsilon literal is allowed
to appear, and everything else refers to it by name. Native, kernel-
constructed geometry is held to a strict nanometer-scale tolerance for point
coincidence and planarity. Geometry arriving through an importer is held to
a separate, deliberately wider planarity tolerance, because coordinates from
a single-precision source format are quantized far more coarsely than that —
a face a user drew flat can arrive a fraction of a millimeter off its own
best-fit plane for reasons that have nothing to do with whether it's
actually warped. Using one strict tolerance everywhere would either reject
huge fractions of real-world imported models outright, or loosen native
geometry's own gate to the point that a native operation producing a
genuinely non-planar face would go undetected; keeping the two tolerances
separate avoids both failure modes.

### 5.5 Determinism and replay

Kernel crates deliberately avoid iteration-order-dependent collections, so
that the same sequence of operations always yields the same result down to
the byte — this is what makes the native file format's own
byte-for-byte determinism (5.6) possible, and what makes a recorded sequence
of operations a reliable regression test rather than one that only
reproduces a bug intermittently.

### 5.6 A layered test strategy

Geometric correctness is tested at the layer where it is cheapest to reason
about, from the bottom up:

- **Property-based tests** at the kernel layer assert invariants that must
  hold for *any* valid input, not just a handful of example cases: extruding
  a closed profile always yields a watertight solid; push/pull followed by
  its exact inverse restores the original topology; splitting a face and
  re-merging the split is the identity operation.
- **Deterministic replay** exercises recorded operation sequences headlessly
  against the same kernel a real session would drive, without a browser or
  GPU in the loop, asserting against a known-good result hash.
- **End-to-end tests** are kept comparatively thin, reserved for the flows
  that only exist once the real UI, rendering, and input pipeline are all in
  the loop together.

Loading the native file format itself runs the same validator a live
mutation does, so a corrupted or hand-tampered file is rejected outright
rather than loaded into a document that looks fine until something touches
the wrong triangle.

### 5.7 History replay is guard-exempt, and carries proof

Some operations run best-effort **obstruction guards**: heuristics that
refuse a sweep whose result would stay manifold — and therefore pass the
topology validator — while self-intersecting in space (a recess pushed
deeper than the material beneath it, a boss driven through geometry in
front of it). Guards read the geometry *surrounding* an operation, and they
are deliberately conservative: refusing a legal forward operation costs the
user a retry, while accepting an illegal one silently corrupts the model.

That conservatism must never break undo. Undo and redo are LIFO — a
recorded inverse dispatches against exactly the state its operation
produced, and a redo dispatches against exactly the state its operation was
originally accepted in — so the state a replayed operation re-enters is one
the kernel has *already accepted*. There is nothing left for a heuristic to
protect against, and a heuristic must not be able to refuse the replay. The
History therefore dispatches recorded inverses (undo) and recorded forward
ops (redo) in a distinct **replay mode**:

- **Guard-exempt.** The obstruction heuristics are skipped. Everything
  structural still runs: input checking, the surgery's own typed refusals,
  the debug invariant checker, and the always-on release validator. Replay
  mode never disables validation — only the heuristics whose job the proof
  below does better.
- **Proof-carrying.** The exemption is not taken on faith. When an entry is
  recorded, the History captures a geometric fingerprint of the state the
  replay must reproduce (for an inverse, the state before the op; for a
  redo, the state after it): every face's outer and hole rings plus its
  plane. The replayed op runs on a clone and commits only if its result
  matches that fingerprint, up to the same tolerance-aware equivalence the
  round-trip property tests use. A mismatch is a kernel bug by definition,
  surfaced as a typed error with the object untouched — undo can fail typed
  on a kernel bug, but it never corrupts, and it is never refused by a
  heuristic.
- **Exact, not merely close.** On a successful match the committed clone is
  aligned to the recorded coordinates (vertex positions and face planes).
  A replayed op recomputes geometry, and floating-point round-trips are not
  bitwise (`(p + d) − d ≠ p`); committing recomputed coordinates would let
  that noise accumulate across undo/redo cycles — refit normals amplify it
  by sweep-distance over face-extent — until a marginal tolerance decision
  inside a later replay flips and refuses an op its forward pass accepted.
  Alignment makes every replay re-enter the exact bits of the recorded
  accepted state, so replay is idempotent and noise cannot accumulate. This
  is not geometry repair (§5.3 is about masking invalid results); it is the
  definition of undo/redo, and the aligned state still passes the full
  validator before committing.

This is DEVELOPMENT.md rule 9. The alternative — holding every guard to one
shared fidelity and trusting that a replayed op re-derives the same
accept/refuse decision at replay time — was rejected because it cannot be
made exact: guards compare tolerances against recomputed positions,
floating-point round-trips are not bitwise (`(p + d) − d ≠ p`), and every
future guard would re-open the gap. Verifying the replayed *result* against
the recorded state discharges the guards' entire purpose without depending
on how any guard is implemented.
