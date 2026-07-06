# Developing Hew

This guide is for anyone building Hew from source or contributing code. It
covers environment setup, repository layout, the engineering rules every
change is held to, and how work in each part of the stack is expected to
flow.

## 1. Prerequisites & setup

Hew is a Rust workspace (the kernel and its supporting crates) plus a
TypeScript/React application that consumes the kernel through a WASM
binding. Building it end to end needs:

- **Rust**, stable channel. The exact toolchain is pinned in
  `rust-toolchain.toml` (`stable`, with the `clippy` and `rustfmt`
  components and the `wasm32-unknown-unknown` target); if you use `rustup`,
  running any `cargo` command from the repository root installs the right
  toolchain automatically.
- **`wasm-pack`**, to compile the kernel to WebAssembly:
  ```sh
  curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
  ```
- **Node.js** and **pnpm**. The UI is a pnpm workspace (`app/`, `shells/*`).
- **Tauri's platform prerequisites**, if you're building the desktop shell
  (a system WebView plus the usual native build tools). See the [Tauri
  prerequisites guide](https://tauri.app/start/prerequisites/) for your OS.

Clone the repository, then:

```sh
cargo build --workspace          # build every Rust crate
pnpm install                     # install JS/TS dependencies
wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg
pnpm --dir app dev               # UI dev server (web build)
pnpm --dir shells/tauri dev      # desktop shell dev
```

The WASM build step is required before the UI dev server will run — the app
imports the kernel bindings from `app/src/wasm/pkg`, which is generated, not
checked in. Re-run it whenever you change a kernel crate the bindings
depend on (`kernel`, `inference`, `tessellate`, `wasm-api`).

Before sending a change, run the full verification script:

```sh
scripts/verify.sh
```

This is the same gate CI runs: Rust formatting and Clippy (with warnings
denied), the whole Rust test suite, a release WASM build, and the app's
type-check/test/build steps. See for the full breakdown.

## 2. Repository layout

```
crates/kernel/        half-edge mesh, Objects, sticky-geometry rules,
                      watertightness tracking, booleans, undo, serialization
crates/inference/     snapping/inference engine (pure geometry queries)
crates/tessellate/    kernel topology -> render buffers
crates/mesh-heal/     healing passes for imported foreign meshes
crates/dae-import/    COLLADA .dae -> Objects/Instances
crates/gltf-import/   glTF -> Objects/Instances
crates/skp-import/    SketchUp .skp -> Objects/Instances, built on the
                      clean-room OpenSKP reader (see rule 7)
crates/wasm-api/      wasm-bindgen surface exposing the kernel to the UI
crates/vendor/        vendored third-party code (dae-parser)
app/                  TypeScript UI (viewport, tools, panels, E2E suite)
shells/tauri/         desktop shell
shells/web/           static web build
tools/                replay runner and development tooling
site/                 the hew3d.com website (Astro)
docs/                 architecture, file format spec, roadmap, this guide
scripts/              verify.sh, the pre-merge verification gate
brand/                logo, mark, and icon assets
```

**Interop crates** all follow the same shape — parse a foreign format into
Hew's Objects/Instances, then hand off to the kernel:

- `dae-import` reads COLLADA 1.4 (`.dae`), the format every SketchUp version
  can export to, via a vendored `dae-parser`.
- `gltf-import` reads glTF 2.0 (`.gltf`/`.glb`).
- `skp-import` reads SketchUp's native `.skp` format directly, using
  [OpenSKP](https://github.com/hew3d/openskp) (crates.io: `openskp`), a
  clean-room implementation with no SketchUp SDK in its history (rule 7).

A STEP/IGES path is planned as an out-of-process OpenCASCADE converter
(a Tauri sidecar rather than a linked crate, because OpenCASCADE is a
heavyweight C++ dependency the kernel has no business depending on); see
`docs/ROADMAP.md`.

The native `.hew` file format is an open, documented container (a zip
holding a JSON manifest and binary geometry buffers). Its spec lives in
`docs/HEW_FILE_FORMAT.md` and any serialization change must update that spec
in the same commit.

## 3. The rules

These are non-negotiable. They are cited from code comments as
"DEVELOPMENT.md rule *N*" — the numbering is load-bearing; do not renumber
this list.

1. **Kernel purity.** `crates/kernel`, `crates/inference`, and
   `crates/tessellate` never depend on UI, rendering, filesystem, or network
   crates. The WASM boundary lives only in `crates/wasm-api`.
2. **Invariant checking.** Every public kernel mutation runs the topology
   validator in debug builds (manifold edges, closed face loops, consistent
   watertightness flags, no orphan half-edges). A new mutation ships with
   validator coverage for whatever invariant it establishes, in the same
   change.
3. **Property-based tests.** Geometric operations are backed by
   property-based tests (via `proptest`), not just examples. At minimum:
   extruding a closed profile is watertight; push/pull followed by its
   inverse restores the prior topology; splitting a face and re-merging it
   is the identity operation.
4. **No silent geometry repair.** If an operation would produce invalid
   topology, it fails loudly with a typed error. The kernel never welds,
   drops, nudges, or reorders geometry to force an operation to succeed.
5. **Tests pass before merge.** No change lands with a failing test or a
   Clippy warning. A failing test is never weakened or deleted to make it
   pass without an explicit, reviewed decision to do so.
6. **Units and precision.** All kernel lengths are `f64` meters. Tolerances
   are named constants in `kernel::tol` — inline epsilon literals are not
   permitted anywhere geometry is compared.
7. **Licensing wall.** Nothing derived from the SketchUp SDK — headers,
   constants, or format knowledge obtained under Trimble's license — enters
   this repository or any part of its dependency chain. That admissibility
   test is the rule, full stop. [OpenSKP](https://github.com/hew3d/openskp)
   (crates.io: `openskp`), the reader behind `crates/skp-import`, is
   derived solely from self-authored test corpora, their own COLLADA
   exports, and public documentation of the MFC `CArchive` serialization
   convention — no SketchUp SDK anywhere in its history. It passes this
   test and is the only sanctioned way `.skp` support exists in this
   project. Anything SDK-derived stays out, permanently, in any project
   Hew depends on.
8. **Discuss before.** Adding a new dependency to a kernel crate, changing
   the native file format, changing the public `wasm-api` surface, or any
   refactor that crosses crate boundaries is proposed and discussed (an
   issue or a PR description laying out the change) before it lands — never
   landed silently inside an unrelated change.

## 4. Kernel development

Kernel work is spec-first. The contract for an operation is written before
its implementation: a doc comment describing behavior, typed error cases,
tolerance obligations, and transactionality, paired with tests in
`crates/kernel/tests/op_specs.rs`. Implementing an operation means making an
existing, already-reviewed contract true — not designing one on the fly.

**The workflow for implementing an operation:**

1. Read the operation's doc comment as its contract.
2. Find its (initially `#[ignore]`d) tests in `op_specs.rs`. Un-ignore them
   in the same change that implements the operation — they are the
   acceptance criteria, not an afterthought.
3. Implement. The doc comment and the specs are fixed points: if one seems
   wrong, that's a discussion to have (rule 8), not something to route
   around by editing the spec to fit the code.
4. End every public mutation with the debug-build invariant validator. If
   the operation establishes an invariant the validator doesn't check yet,
   extend the validator in the same change (rule 2).
5. Before committing:
   ```sh
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   cargo fmt --check
   ```

**Standing guarantees, and why they're worth the cost:**

- **Strong exception guarantee.** On `Err`, a mutating operation leaves its
  Object exactly as it was before the call. The straightforward
  implementation — clone, mutate the clone, validate, then swap — is
  correct by construction; only optimize it with a benchmark in hand, never
  by weakening the guarantee.
- **Tolerances are named constants.** Only the constants in `kernel::tol`.
  Never compare floats with `==` for geometric meaning, and never call
  `.normalized().unwrap()` on a vector that might be degenerate — treat
  that case as a typed error.
- **No silent repair** (rule 4). If input, or an intermediate state inside
  an operation, is invalid, return the typed error rather than adjust the
  geometry to compensate.
- **Handles are generational.** Deleting and re-creating an element changes
  its id; a stale handle simply fails to look anything up rather than
  aliasing something else. Reports can return now-dead handles in their
  `removed_*` fields — that's safe by construction. Never cache a handle
  across a mutation from inside another operation's own implementation.

**Traps worth knowing before you hit them:**

- Outer loops wind counter-clockwise seen from the face normal; a twin pair
  is the same undirected edge traversed in opposite directions
  (`twin.origin == origin(next(h))`).
- Moving geometry invalidates any `Plane` derived from it. Refit planes from
  the boundary (Newell's method) rather than translating a stored plane's
  offset by hand, and remember to refit the *neighboring* faces whose
  boundaries moved too.
- Never transform a normal with the same matrix used for points; normals
  need the inverse-transpose, which is why planes cross transforms through
  a dedicated helper rather than the generic point/vector transform.
- Faces with inner loops are annuli, not disks — the naive Euler formula
  (`V − E + F = 2`) does not hold for them. Use the Euler–Poincaré form that
  accounts for hole loops before assuming a face/edge count is wrong.
- Floating-point round-trips are not bitwise identical
  (`(p + d) − d ≠ p` in general); topology-identity tests compare via
  tolerance-aware equivalence, not exact equality, and new tests should
  hold to the same standard.
- Two faces can each be planar within tolerance against their *own* fitted
  plane and still fail to merge — coplanarity for a merge should be judged
  against one plane refit over the union of both boundaries.

When something doesn't add up — a spec test looks wrong, you need a new
tolerance constant or a public API change, the validator fails and you
don't know why, or you're tempted to mark a test `#[ignore]` or loosen an
assertion "temporarily" — stop and raise it (rule 8) rather than improvise.
A kernel that is honestly incomplete is worth more than one that is
quietly wrong.

## 5. The WASM boundary

`crates/wasm-api` is the only place the kernel is exposed to the outside
world (rule 1), and its public surface changes only with the discussion
rule 8 requires. The boundary follows a small set of conventions; extend
it by matching them, not by inventing new ones per method.

- **State lives inside the module.** The kernel's `Document` is the
  authoritative model. The WASM-side `Scene` is a thin shim over it, adding
  only what the kernel itself may not depend on — the inference engine and
  per-Object render-mesh caches. Every mutation goes: call into `Document`,
  then reconcile those caches from what changed. Geometry does not cross
  the boundary on every frame, only on commit.
- **Handles are opaque `u64`s** (JS `BigInt`), reusing the kernel's
  generational slotmap keys. JavaScript stores and passes them back but
  never does arithmetic on them. A stale handle raises a typed error, never
  a crash.
- **Errors are thrown as `JsError`s shaped `CODE: message`.** The code is
  the kernel's typed error variant name, so callers can switch on it
  without parsing prose.
- **Buffers are copied out, never aliased.** Getters return fresh typed
  arrays (`Float32Array`, `Uint32Array`, …); zero-copy views into WASM
  memory are rejected as a category, because memory growth silently
  invalidates them. Expensive results (tessellation) are cached on the Rust
  side and invalidated on mutation, so the copy is cheap in the common case
  of repeated reads with no change in between.
- **Reports are dedicated binding classes** with getter methods, mapping
  1:1 onto the kernel's own report structs — no ad hoc JSON blobs.

**Adding to the surface safely:** write the new method's contract the same
way a kernel operation gets one — inputs, error cases, what it mutates —
and get it discussed before merging (rule 8). Prefer exposing the smallest
read-only accessor that solves the problem over a broad, speculative one;
prefer routing a new capability through an existing report/reconcile path
over adding a bespoke side channel. The people who need the full method
list can read `crates/wasm-api/src/lib.rs` directly; this document is about
the shape new additions should take, not an API reference.

## 6. UI development

The TypeScript app drives the kernel through the WASM boundary; the tool
layer is where user gestures become kernel calls.

**Core principle: the kernel owns truth, tools own gestures.** A tool
accumulates pointer state (a rubber-band rectangle, a preview extrusion
height) entirely as viewport ephemera, and commits it via a single `Scene`
call only when the gesture completes. Nothing partial — a half-drawn
polyline, an unconfirmed push/pull — ever reaches the kernel.

**Tool architecture:**

```ts
interface Tool {
  onPointerMove(snap: Snap | null, ray: Ray): void   // update preview/cues
  onPointerDown(snap: Snap | null, ray: Ray): void    // advance the gesture
  onKey(ev: KeyboardEvent): void                      // esc cancels a stage
  cancel(): void                                      // full reset
  readonly name: string
}
```

A `ToolController` holds exactly one active `Tool` and routes pointer/key
events to it; switching tools cancels the outgoing one first. Camera
navigation (orbit/pan/zoom) is deliberately *not* a tool — it is always
live, bound to its own input (middle-drag, scroll), and never competes with
whatever tool is active.

- **Every pointer move is resolved through the inference/snap query**
  first; a tool only falls back to intersecting a plane itself when
  inference has nothing to offer. Some tools additionally supply a snap
  constraint (an axis lock, or a plane restricted to the face being drawn
  on) that the viewport feeds into the next snap query — features are
  opt-in and detected structurally rather than through a shared base class.
- **Commit, then re-pull.** After a `Scene` mutation, the app re-reads
  whatever geometry the mutation's report says changed (mesh, sketch
  lines, regions) and refreshes the affected UI (watertight badge, undo
  button state). There is no optimistic local geometry — the render always
  reflects what the kernel actually holds.
- **Typed kernel errors surface as toasts**, keyed off the error's `CODE:`
  prefix. Because mutations carry the kernel's strong exception guarantee
  (), handling an error is just clearing the tool's preview state — there
  is never partial state to unwind.

**Testing the UI:** component-level behavior is covered with component
tests (Vitest + Testing Library) alongside the components they exercise.
For behavior that spans the kernel, inference, and the tool layer together,
the app exposes a semantic test harness, `window.__hew_test`, in debug/test
builds only. It lets a driver (a Playwright test, or a console) issue
actions in terms of what a user is doing — draw a rectangle, push/pull a
face, undo, set the camera — and read back state (object counts, selection,
a canonical state hash) without touching the canvas at pixel level. Handles
and other 64-bit values cross this boundary as decimal strings rather than
`bigint`, because `bigint` isn't structured-cloneable out of a browser
automation context and plain numbers lose precision above 2^53; the
harness converts internally. Driving tests through this harness rather than
synthesized pointer events keeps them fast and immune to layout/pixel
flakiness, reserving raw pointer-driven tests for the cases that
specifically need to validate screen-to-world picking.

## 7. Testing & verification

Hew's test suite is a pyramid, deliberately weighted toward its fastest,
most deterministic layers:

| Layer | What it covers | Where |
|---|---|---|
| Rust unit + spec tests | Kernel operation contracts (`op_specs.rs`) | `crates/kernel`, `crates/inference` |
| Property-based tests | Geometric invariants across random inputs | same crates, via `proptest` |
| Component tests | UI component behavior in isolation | `app/`, via Vitest + Testing Library |
| Headless replay | Command-stream replay against a golden state hash, no browser GPU required | `crates/wasm-api` (Node target) |
| End-to-end | Full browser/webview smoke flows, driven mostly through `__hew_test` | `app/` E2E suite (Playwright) |

Most interaction-logic regression coverage belongs in the middle layers —
kernel property tests and headless replay — because they're fast and
immune to rendering flakiness. End-to-end tests stay few and high-value:
smoke-level flows that validate real wiring (rendering, input, the desktop
shell), not exhaustive feature coverage.

**`scripts/verify.sh`** is the single gate a change must pass before it
merges, and it's what CI runs:

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg
pnpm --dir app typecheck && pnpm --dir app test && pnpm --dir app build
pnpm --dir site check && pnpm --dir site build
```

It also holds the desktop shell crate to the same formatting/Clippy bar,
even though it isn't a workspace member (it carries desktop-only
dependencies that don't belong in the kernel test loop).

**Determinism and replay.** The kernel is required to be bit-for-bit
deterministic — the same input produces the same output on every machine,
including iteration order over any internal collection. This is what makes
recorded sessions replayable as regression tests: a reported bug's input
stream, once captured, becomes a permanent, exact reproducer rather than a
"works on my machine" anecdote. See `docs/DIAGNOSTICS.md` for the full
determinism, logging, and record/replay architecture.

