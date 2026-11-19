# Developing Hew

This guide is for anyone building Hew from source or contributing code. It
covers environment setup, repository layout, the engineering rules every
change is held to, and how work in each part of the stack is expected to
flow.

## 1. Prerequisites & setup

Hew is a Rust workspace (the kernel and its supporting crates) plus a
TypeScript/React application that consumes the kernel through a WASM
binding. Building it end to end needs:

Every toolchain below is pinned to an exact version in a file in this
repository, and CI installs from those same files. §8 covers why that is
non-negotiable and how the pins get bumped.

- **Rust.** The exact toolchain is pinned in `rust-toolchain.toml` (a specific
  release, with the `clippy` and `rustfmt` components and the
  `wasm32-unknown-unknown` target). If you use `rustup`, running any `cargo`
  command from the repository root installs it automatically — so you cannot
  accidentally compile with a different compiler than CI does.
- **`wasm-pack`**, to compile the kernel to WebAssembly. The pinned version
  installs with:
  ```sh
  scripts/install-wasm-pack.sh
  ```
- **Node.js**, at the version in `.node-version`. `fnm` reads that file
  natively, as do GitHub Actions' `setup-node` and Cloudflare Pages. Two
  common managers do not, and will silently leave you on whatever Node your
  shell already had: plain `nvm` only looks for `.nvmrc`, so use
  `nvm use "$(cat .node-version)"`; `asdf` needs `legacy_version_file = yes`
  in `~/.asdfrc` before it will read it.
- **pnpm.** The UI is a pnpm workspace (`app/`, `shells/*`). Its version lives
  in the root `package.json`'s `packageManager` field and pnpm switches itself
  to it, so how you install pnpm in the first place does not matter.
- **Tauri's platform prerequisites**, if you're building the desktop shell
  (a system WebView plus the usual native build tools). See the [Tauri
  prerequisites guide](https://tauri.app/start/prerequisites/) for your OS.

Clone the repository, then:

```sh
scripts/install-wasm-pack.sh     # the pinned wasm-pack
pnpm install                     # install JS/TS dependencies
pnpm --dir app dev               # UI dev server (web build)
pnpm --dir shells/tauri dev      # desktop shell dev
```

The app imports the kernel bindings from `app/src/wasm/pkg`, which is
generated, not checked in. `pnpm --dir app dev` and `pnpm --dir app build`
build it first when it is missing, older than its sources, or the wrong
flavor for the command (`app/scripts/build-wasm.mjs`). The dev server is a
preview build unless `HEW_PREVIEW=0` is set, so it gets the preview kernel;
see [Preview features](#preview-features). The first build compiles the
kernel crates and takes several minutes. `cargo build --workspace` builds
every crate natively and is not needed to run the app.

Before every commit, run the fast verification script:

```sh
scripts/verify.sh
```

It covers Rust formatting and Clippy (with warnings denied), the whole Rust
test suite, a WASM build of the kernel (without `wasm-opt`; CI builds the
size-optimized module that ships), the app's type-check/test/build steps, and
the Tauri host crate's fmt/clippy. It is deliberately dependency-light and
fast, so it does **not** run the browser- and runner-heavy CI lanes — the
replay gate and the Playwright E2E suite. A green `verify.sh` is therefore
necessary but not sufficient for a green CI run.

While a change is still in progress, there is a quicker check that runs
only what the change can reach:

```sh
scripts/verify.sh --changed        # or: scripts/verify-changed.sh [base]
```

It compares the working tree with the merge base on `main` (or the branch
you name) and runs the checks for what differs: Clippy and tests for the
changed crates and the crates that depend on them, the app's type-check,
lint and the unit tests related to the changed files, a handful of smoke
specs plus any spec you edited (the visual goldens aside), and so on. An edit to one small crate or
one app file takes a minute or two; a kernel edit still rebuilds and
tests every crate downstream of it. It skips whatever the change does not
touch, so it does not replace `verify.sh`: run that before the change is
done. The header of `scripts/verify-changed.sh` lists what each kind of
change runs and what is left to the gate.

Before **pushing**, run the comprehensive gate:

```sh
scripts/verify-full.sh
```

It runs `verify.sh`, the dependency advisories, the golden state-hash
replay gate, the relay contract, and the chromium E2E **smoke set** in both
builds: the specs listed in `scripts/e2e-smoke-specs.txt`, the Preview wall
spec, and every spec file the branch changed.

The whole chromium suite is CI's job. It is serial by design and takes well
over an hour on one machine; CI spreads it over eight runners and blocks the
merge on it. Run it locally when a change is wide enough to earn the wait
(the input pipeline, the tool state machines, the test harness, a Playwright
bump):

```sh
scripts/verify-full.sh --full-e2e
```

A green run predicts a green CI run with two documented exceptions. The
specs outside the smoke set run only in CI, unless `--full-e2e` is given.
And the **visual-goldens lane blocks in CI but cannot run here**, because its
PNGs are rasterized by the SwiftShader inside CI's pinned Chromium and
false-fail on any other host. If you touched the render path — or bumped
three.js or Playwright — CI has the last word. The gate also skips the
non-blocking webkit run (nightly in CI) and the Desktop E2E workflow
(tauri-driver cannot drive the macOS WKWebView). The header of
`scripts/verify-full.sh` carries the full rationale.

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
                      clean-room Skpwright reader (see rule 7)
crates/stl-import/    STL (binary + ASCII) -> Objects; hand-written parser,
                      no external STL dependency
crates/image-import/  PNG/JPEG -> a placed picture (one painted face in a
                      component, the .skp image shape); reads only the size
crates/hlr/           hidden-line removal: visible line work of a view as 2D
                      segments (+ SVG), for vector Line-art printing and export
crates/headless-render/ the renderers behind hew.view.snapshot and
                      hew.view.line_drawing, shared by hew-cli and the app's
                      live host (over softrender and hlr)
crates/pdfwrite/      hand-written PDF 1.4 writer for Save PDF… and the
                      headless hew.print.pdf
crates/wasm-api/      wasm-bindgen surface exposing the kernel to the UI
crates/hew-relay/     self-hostable "Open on Phone" relay binary (axum;
                      speaks workers/share-relay's contract)
crates/library/       the Hew Library's on-disk layout (folder resolution,
                      item naming, atomic list/read/write/remove), shared
                      by hew-cli's hew.library.* and the desktop shell
crates/plugin-manifest/ the plugin manifest (hew-plugin.json) parser and
                      validator; pure, shared by every host and crates/api
crates/vendor/        vendored third-party code (dae-parser)
app/                  TypeScript UI (viewport, tools, panels, E2E suite)
shells/tauri/         desktop shell
shells/web/           static web build + deploy/ (nginx, relay stanza,
                      systemd unit — shipped inside the release tarball)
workers/share-relay/  the public relay Worker, its unit suite, and the
                      black-box conformance suite both relays must pass
workers/bug-intake/   the private Help ▸ Report Bug intake Worker (chunked
                      upload, Access-protected admin page) and its unit suite
tools/                replay runner and development tooling
corpus/curves/        the curves scoreboard: test models as API scripts and
                      the checked-in result table (section 7)
site/                 the hew3d.com website (Astro)
docs/                 architecture, file format spec, roadmap, this guide
scripts/              verify.sh (per-commit gate), verify-changed.sh (quick
                      check of a change in progress), verify-full.sh
                      (pre-push), relay-contract.sh (relay conformance,
                      both servers)
brand/                logo, mark, and icon assets
```

**Interop crates** all follow the same shape — parse a foreign format into
Hew's Objects/Instances, then hand off to the kernel:

- `dae-import` reads COLLADA 1.4 (`.dae`), the format every SketchUp version
  can export to, via a vendored `dae-parser`.
- `gltf-import` reads glTF 2.0 (`.gltf`/`.glb`).
- `skp-import` reads SketchUp's native `.skp` format directly, using
  [Skpwright](https://github.com/hew3d/skpwright) (crates.io: `skpwright`), a
  clean-room implementation with no SketchUp SDK in its history (rule 7).
- `stl-import` reads STL (binary and ASCII, auto-detected), the format the
  maker community lives in. Hand-written on `std` — no external STL crate —
  since the binary layout is a fixed-width record and the ASCII grammar is a
  handful of whitespace-separated tokens. STL carries no units, so the
  crate is unit-blind (the caller resolves a `unit_scale`); its stored
  facet normals are unreliable and are ignored, same as every other
  importer's foreign normals.
- `image-import` places a PNG or JPEG as a picture, in the shape a `.skp`
  image arrives in: one face painted with the picture inside a component
  named after the file. It reads only the picture's size from its header
  (plus a JPEG's EXIF orientation); the bytes are kept as they are.

A STEP/IGES path is planned as an out-of-process OpenCASCADE converter
(a Tauri sidecar rather than a linked crate, because OpenCASCADE is a
heavyweight C++ dependency the kernel has no business depending on); see
`docs/agents/ROADMAP.md`.

The native `.hew` file format is an open, documented container (a zip
holding a JSON manifest and binary geometry buffers). Its spec lives in
`docs/dev/HEW_FILE_FORMAT.md` and any serialization change must update that spec
in the same commit.

**The website** (`site/`) is built by Cloudflare Pages from the `site-live`
branch, not from `main`, so hew3d.com describes the release people can
download while `main` carries the Learn pages for features that have not
shipped yet. Docs for a feature still land on `main` in the feature's own
commit; nothing changes about how you write them. `site-live` is a pointer,
never a branch anyone commits to: the release workflow moves it to each
`vX.Y.Z` tag, and only forward, judged by the version in
`site/src/data/releases.json`, so an out-of-order or re-run release never
rewinds it. A `vX.Y.Z-pre.LABEL` pre-release tag (a build of an effort
that has not shipped, cut from that effort's integration branch) builds
every package but never moves it. The job runs from the workflow file at the tagged commit, so a
patch tag cut from an older release branch moves the pointer only if that
branch carries the job. With Pages' default preview setting, a push to `main`
only produces a preview deployment at `https://main.<pages-project>.pages.dev`
(the project name is in the Cloudflare dashboard), which is the user guide for
the next release and the place to proofread it. A site fix that cannot wait
for the next release goes to `main` first, then onto the branch from a
throwaway worktree based on the remote tip, never on a stale local branch:

```sh
git fetch origin
git worktree add -B site-live ../hew-site-fix origin/site-live
# cherry-pick the fix, then a plain push; a rejected push means a stale base
```

The next release moves the pointer to its tag, so the fix must be in that
tag's history: if the release is cut from a release branch rather than
`main`, cherry-pick the fix there too.

**The beta web app** (beta.hew3d.com) is a second Cloudflare Pages project,
`hew-app-beta`, separate from the release app at app.hew3d.com. The
`deploy-beta` job in `.github/workflows/ci.yml` deploys it on every push to
`main`, once the gate jobs (`rust`, `web`, `tauri`, `replay`, `e2e-chromium`, `e2e-visual`, `relay-contract`, `bug-intake-tests`)
are green. Builds that deploy there set `HEW_BETA_SHA`, which stamps the
version as `<version>+beta.<sha>` and switches on the in-app beta cues and
the "Hew Beta" PWA name. Releases, local builds, E2E builds and the
self-hosting tarball never set it, so they are unchanged. The two Workers
that serve same-origin routes, share-relay and bug-intake, carry
`beta.hew3d.com` routes in their `wrangler.toml`; redeploy each with
`npx wrangler deploy` whenever those routes change. The beta is deliberately
unlinked from the website and the README, so do not add links to it there.

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
   There is one exception, and it is bounded and reported: where a
   boolean's own cut leaves a piece narrower than `tol::FEATURE_MIN`
   (0.1 µm), the boolean merges that piece into its neighbors. It moves no
   point farther than `tol::TIDY_REACH` (0.2 µm), moves only points within
   that reach of such a piece, and returns what it merged, and where, in
   its result and in the log. A tidied result must pass every check an
   untidied one does; one that cannot is discarded, and the boolean
   answers as it would have without the tidy. Geometry the cut does not
   touch is never changed, and no other operation may merge geometry this
   way without a change to this rule.
5. **Tests pass before merge.** No change lands with a failing test or a
   Clippy warning. A failing test is never weakened or deleted to make it
   pass without an explicit, reviewed decision to do so.
6. **Units and precision.** All kernel lengths are `f64` meters. Tolerances
   are named constants in `kernel::tol` — inline epsilon literals are not
   permitted anywhere geometry is compared.
7. **Licensing wall.** Nothing derived from the SketchUp SDK — headers,
   constants, or format knowledge obtained under Trimble's license — enters
   this repository or any part of its dependency chain. That admissibility
   test is the rule, full stop. [Skpwright](https://github.com/hew3d/skpwright)
   (crates.io: `skpwright`), the reader behind `crates/skp-import`, is
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
9. **History replay is guard-exempt, with proof.** A recorded inverse
   (undo) or recorded forward op (redo) re-enters a state the kernel
   already accepted, so the best-effort obstruction heuristics that forward
   operations run are skipped on replay; instead, the replayed result is
   verified against a geometric fingerprint of the recorded state (face
   rings, hole rings, planes) and committed only on a match — after which
   the committed result is ALIGNED to the recorded coordinates, so replay
   restores the accepted state exactly and floating-point noise cannot
   accumulate across undo/redo cycles (see ARCHITECTURE.md §5.7). A
   mismatch fails typed with the object untouched — undo may fail typed on
   a kernel bug, but it is never refused by a heuristic and it never
   corrupts. Structural validation is never skipped on replay. A heuristic
   that can refuse a replay, or a test harness arm that tolerates a failing
   undo, is a bug — not something to whitelist.

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
  a per-Object record of where each face sits in its render mesh (the
  render buffers themselves are the app's to keep: on a large model they
  are hundreds of megabytes the kernel's memory cannot spare twice). Every
  mutation goes: call into `Document`, then reconcile those caches from
  what changed. Geometry does not cross the boundary on every frame, only
  on commit.
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

**UI text is localized.** Every string a person reads goes through a
Lingui macro (`t`, `<Trans>`, `msg`) and into the gettext catalogs under
`app/src/locales/`; `pnpm --dir app i18n:extract` regenerates them and the
verify gate fails if they are stale. A shipped locale may lag: untranslated
messages show in English and the gate only reports the coverage.
`pnpm --dir app lint` (Oxlint with the Lingui plugin) rejects an unwrapped
string. Numbers a person reads go through `app/src/i18n/format.ts`, never
`toFixed`. Strings in `crates/` stay English: the kernel speaks in typed
error codes and structured history kinds, and the app owns the words.
`docs/LOCALIZATION.md` is the full contract, for contributors and
translators both.

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
HEW_PREVIEW=1 cargo clippy / cargo test   (the crates the flag changes)
scripts/wasm-flavors.sh build production web --no-opt; ... build preview web --no-opt
pnpm --dir app typecheck && pnpm --dir app test (both builds) && pnpm --dir app build
pnpm --dir site check && pnpm --dir site build
```

It also holds the desktop shell crate to the same formatting/Clippy bar,
even though it isn't a workspace member (it carries desktop-only
dependencies that don't belong in the kernel test loop). Every step the
Preview flag changes runs twice, once per build (see below).

### Preview features

Unfinished features live on `main` behind a build-time flag. A preview
build has them; a production build, the one regular users get, has no way
to reach them: no tool, menu item, palette entry, keyboard shortcut, dock
chip or API command. The flag is fixed when Hew is built, so there is
nothing to switch on in a production build.

**The flag.** `HEW_PREVIEW=1` in the build environment makes a preview
build. Any other value, or none, makes a production build. It reaches:

- `crates/api` (its `build.rs` sets `cfg(hew_preview)`), and through it
  `hew-cli`, the MCP server, the WASM module's live API, and the plugin
  host's dispatch;
- `crates/wasm-api` and `crates/hew-cli`, each with a `build.rs` of its
  own that sets the same cfg, so an export or a subcommand of a feature
  that loads code can be left out of a production build. Each crate fails
  to compile if its flag and the API crate's disagree;
- the desktop shell (`shells/tauri/src-tauri/build.rs`), for the native
  menu and the updater: a preview build never updates itself;
- the app, as `__HEW_PREVIEW__` (`app/vite.config.ts`). The dev server
  defaults it to on and `vite build` to off; set `HEW_PREVIEW=0` or `1` to
  choose.

**One rule for the desktop shell and its page.** The flag has one
resolution, and `shells/tauri/scripts/tauri-policy.mjs` applies it for the
desktop shell: `1` is a preview build, `0` a production build, anything
else an error, and unset or empty is a preview build for `dev` and a
production build for `build`. The wrapper exports the answer (`1` or `0`)
as `HEW_PREVIEW` to the Tauri CLI, so the Rust shell
(`cfg(hew_preview)`), the page `vite dev` or `vite build` serves and the
policy overlay can never disagree, even when the caller's variable was
unset. `--print-args` prints what the wrapper would run, which
`scripts/check-production-artifacts.mjs` and the wall tests read. Use the
shell's package scripts only: the Tauri CLI run directly (`pnpm tauri`,
`cargo tauri`) applies neither the overlay nor the flag. A preview shell
started with a policy that does not allow the realm's boot script logs an
error and refuses every plugin command.

A `vX.Y.Z-pre.N` tag builds every package with the flag on, publishes a
pre-release with no update feed, and deploys the web app to the `preview`
Pages branch. A production tag never sets it, and the release workflow
fails if it finds it set.

**The registry.** `preview-features.json` at the repo root lists every
Preview feature: an id, a name, a one-line state, an owner, and what it
gates (`tools`, `actions`, `dockChips`, `apiCommands`, `behaviors`, and
for a feature that loads code, `loader`; see "Code loading" below).
Nothing is gated anywhere else. The app reads it in `app/src/preview/`,
`crates/api` in `src/preview.rs`, and the desktop shell in
`src/preview_menu.rs`. In a production build an API command of an entry
is left out of the registry, so it appears in no listing, and a call
naming it is refused with `preview_only`. A behavior that is an API
parameter (Follow Me's `twist`, for one) is bound to its command in
`GATED_PARAMS` in `crates/api/src/preview.rs`; the command stays, the
parameter is refused. A behavior that is a value of a live field (the
Library's `profile` category) is bound in `GATED_VALUES`: the value
leaves the command's schemas, a request naming it is refused, and the
handler neither lists nor touches an item that carries it. Refusal copy a
handler writes that would name a Preview feature lives in
`GATED_API_COPY`, with the production wording beside it; the wall test
fails on any other handler string that names one.

**Code loading.** A feature that loads and runs code (plugins are the
first) has entry points no table holds, so its entry carries a `loader`
object and `"defaultOn": false`. Its switch in Settings ▸ Preview starts
off, and with the switch off a preview build behaves like a production
one for that feature. The lists in `loader`, and how each is kept out of
a production build:

| List | What it names | Production build |
|---|---|---|
| `roots` | window roots picked by location hash in `app/src/main.tsx` | `previewBuild && rootAllowed('<id>')` ahead of the hash test; the root's lazy import is in `app/src/preview/loaderRoots.ts` behind the build literal, so the bundle has neither |
| `panes` | Settings panes | `paneAllowed('<id>')` at the one tab table |
| `shellCommands` | desktop commands | defined under `src/preview_only/`, a module compiled only with `cfg(hew_preview)`, and absent from the handler list every build registers |
| `kernelExports` | exports of the WASM module | `#[cfg(hew_preview)]` in `crates/wasm-api` |
| `cliCommands` | `hew-cli` subcommands | `#[cfg(hew_preview)]` on the match arm, the usage text, the "did you mean" list and the module behind it, so `hew-cli <word> --help` is answered as for any unknown word |
| `storage` | folders, databases and settings-key prefixes | touched only by code in the gated modules |
| `markers` | strings that must not be in a production artifact | scanned for |

`rootAllowed`, `paneAllowed` and `loaderEnabled` (`app/src/preview/`)
answer false for an id no entry lists, the reverse of the rule for tools
and commands: nothing they guard is public in any build. A host that
loads code asks `loaderEnabled('<feature>')` before it starts anything.
The switch gates the app's side only. In a preview build the desktop
commands a `shellCommands` list names answer whether the switch is on or
off; what the switch decides is whether the app's UI and loader call
them.
An event name a loader feature adds is a `GATED_VALUES` row, as for any
other feature. A bus command the desktop shell answers itself instead of
forwarding it to the app (`hew.plugin.list`, `hew.plugin.dev`) is named
in `PREVIEW_HOST_METHODS` in `shells/tauri/src-tauri/src/live_router.rs`,
which is empty in a production build; there it is forwarded like any
other method, and the app's dispatcher answers it from the registry.
A production `hew-cli` does not rely on that: in `--live` mode
(`dispatch`, `run`, `mcp`) `LiveSession::dispatch`, the one place every
live request passes, refuses with `preview_only` any method the registry
marks as a Preview command, bare or inside `hew.doc.transact`, before
anything is sent. A method the registry does not know is forwarded, since
a newer app may serve it. The MCP server of `hew-cli` also refuses every
`hew.plugin.*` method but `hew.plugin.list` to an agent in every build; a
production build leaves a name the registry marks to that `preview_only`
answer and refuses any other with wording that names no feature.
`crates/hew-cli/tests/live_preview_wall.rs` pairs each build of the CLI
with a fake app socket that records what it is sent.

Both shells' content security policies live in
`shells/csp/policies.json`: a `production` and a `preview` policy for
each shell, and the `probe` policy the realm probe is measured under,
which no release serves. `shells/web/inject-csp.mjs` picks the web one by
the flag; `tauri.conf.json` holds the desktop production policy, and
`shells/csp/tauri-overlay.mjs` writes a `--config` overlay for any other.
A Preview feature that needs a wider policy widens the preview one. The
desktop shell's `dev`, `build` and `e2e:build` scripts run the Tauri CLI
through `shells/tauri/scripts/tauri-policy.mjs`, which adds the
`desktop.preview` overlay (written under `src-tauri/target/`) whenever
the app it builds is a preview build (the rule above). The release
workflow passes the same overlay for a `-pre.` tag and no other, and
after the build `scripts/check-desktop-binary.mjs` checks that the
shell binary allows the boot script for a `-pre.` tag and does not for
any other; the check below holds the wiring to that. The
production policies and the desktop capability files are pinned to copies
under `shells/csp/baseline/`, so changing one takes an edit in both
places. Hew sets no cross-origin isolation header on any tier.

The plugin realm's boot script is built from `sdk/plugin` (the
`@hew3d/plugin` workspace package) and committed as
`app/src/plugins/boot/boot-1.js` with its hash in `boot-1.hash`. The
preview policies pin that hash in `script-src`, and the Plugins registry
entry lists it as a marker. Both files are byte-exact (`.gitattributes`
sets `-text` on them), so no checkout rewrites their line endings. After a change to `sdk/plugin/src/`, run
`pnpm --dir sdk/plugin build` and paste the printed hash into
`shells/csp/policies.json` (`web.preview`, `desktop.preview`) and the
entry's `markers`; `app/src/plugins/boot/bootArtifacts.test.ts` fails
until all of them agree with a fresh build.

`scripts/check-production-artifacts.mjs` runs after the production app
build in `scripts/verify.sh`, and in the release workflow for a production
tag. It fails on a production policy or capability file that left its
baseline, an isolation header, a `markers` string or a feature's state
line anywhere in the app build, and a `kernelExports` name in the
production kernel's types. The kernel `.wasm` and the native binaries
embed the registry's text, so they are checked by export and command
name, not by scanning for markers.

**Adding a feature.** Add its entry to `preview-features.json` and build
the feature as usual. The wall tests walk the registry, so the entry is
covered without a test change: `crates/api/tests/preview_wall.rs`,
`crates/hew-cli/tests/preview_wall.rs`, the live-API test in
`crates/wasm-api`, the native menu test in the shell,
`app/src/preview/wall.test.ts`, and `app/e2e/preview-wall.spec.ts`. A
test of the feature itself runs in the preview build only: in Rust,
`#[cfg(hew_preview)]` (in `crates/api`; other crates check
`api::preview::PREVIEW_BUILD`); in a browser spec,
`requiresPreview('<id>')`. Every name in an entry must be real, and the
tests fail on one that is not.

**Graduating a feature.** Delete its entry, then regenerate the API
artifacts (`REGENERATE_API_ARTIFACTS=1 cargo test -p api --test
generate_artifacts`): the reference and the TypeScript SDK mark each
Preview command, so their diff shows what became public. Remove the
feature's `GATED_PARAMS`, `GATED_VALUES` and `GATED_API_COPY` rows, if it
has any, and its `cfg(hew_preview)` and
`requiresPreview` marks. Removing a feature that will not ship is the same
deletion, with its code. Every entry ends one of those two ways. The
registry is read through at each release, and an entry nobody is
working toward releasing is removed.

**Commits and the changelog.** A commit whose work can only be reached
in a preview build takes the type `preview` in place of `feat`, with the
usual scope: `preview(kernel): …`, `preview(app): …`. `feat` is kept for
what a production build gains, so the history never promises a feature
users cannot find. Where such a commit also changes something in every
build, its body says what. Graduating a feature is the `feat` commit.
`CHANGELOG.md` and release notes do not mention a Preview feature until
it graduates; `app/src/preview/guidePages.test.ts` checks the unreleased
changelog, the README, the roadmap and the published guide for the
names it knows.

**The realm probe.** `/#plugin-probe` in a preview build with the Plugins
switch on runs the sandbox measurements the plugin design rests on.
`pnpm --dir app exec playwright test e2e/plugin-realm` runs them in a
browser under the `web.probe` policy; `pnpm --dir shells/tauri
e2e:build:probe` then `e2e:probe` runs them in the desktop webview under
`desktop.probe`.

**Running both builds.** `scripts/verify.sh` and `scripts/verify-full.sh`
run both on their own; nothing in them skips the production half (the
browser smoke set narrows which specs run, never which builds). By hand:

```
cargo test -p api                       # production
HEW_PREVIEW=1 cargo test -p api         # preview
scripts/wasm-flavors.sh build preview web --no-opt
scripts/wasm-flavors.sh use preview     # app/src/wasm/pkg is the preview module
HEW_PREVIEW=1 pnpm --dir app dev
scripts/wasm-flavors.sh build production web --no-opt
scripts/wasm-flavors.sh use production
HEW_PREVIEW=0 pnpm --dir app exec playwright test --project=chromium
```

The WASM module carries the API's live half, so each build has its own
module; `preview_build()` on the module says which one it is. The app's
`dev` and `build` scripts ask the module in `app/src/wasm/pkg` and rebuild
it when it is the other flavor, so `pnpm --dir app dev` and
`HEW_PREVIEW=0 pnpm --dir app dev` each start on the right kernel without
the `wasm-flavors.sh` lines. One slot holds one flavor, so each switch is
a rebuild; `wasm-flavors.sh` keeps both built side by side, as the gate
does, and `use` swaps between them in seconds. The dev server's own
builds skip `wasm-opt`, as the gate's do, and `pnpm --dir app build`
rebuilds over either rather than bundle an unoptimized kernel.

**Determinism and replay.** The kernel is required to be bit-for-bit
deterministic — the same input produces the same output on every machine,
including iteration order over any internal collection. This is what makes
recorded sessions replayable as regression tests: a reported bug's input
stream, once captured, becomes a permanent, exact reproducer rather than a
"works on my machine" anecdote. See `docs/dev/DIAGNOSTICS.md` for the full
determinism, logging, and record/replay architecture.

### The curves scoreboard

The scoreboard says, case by case, what Fillet and Chamfer do on a set of
test models. Every change to the blend engine is judged against it.

```sh
scripts/scoreboard.sh
```

rebuilds every model under `corpus/curves/` through the Hew API, runs every
case at every size of its sweep, rewrites `corpus/curves/SCOREBOARD.tsv`,
and prints what changed since the last run, cases that got worse first.
`scripts/scoreboard.sh --check` runs the same cases and fails if the table
would change, without writing it. Each run of a case at a size gets one
class:

| Class | Meaning |
|---|---|
| `correct` | A valid, watertight solid that passed the checks below |
| `limit` | Refused, and the refusal names what is in the way |
| `generic` | Refused without a cause a user could act on |
| `wrong` | A result that failed a check, a panic, or an undo that did not restore the model |

A built result is `correct` when the object validates, is watertight, lies
inside the bounds it had before, and moved some volume, but no more than
the size squared times the total length of the object's edges. A plain
round or chamfer must also report the size that was asked for. That
catches a broken or badly oversized result. It does not measure the
surface: a round that is valid but slightly off its true shape still reads
`correct` here. A refusal that says a larger size would fit than the one it
refused is `wrong`.

Every edge of a `correct` result, and of every model as built, must also
read the way its geometry says (`check_edge_classes`): a facet seam of one
surface as a seam, a joint between two tangent surfaces (a fillet's rail,
a corner patch's border) as a tangent joint, and everything else, a miter
inside one blend included, as a hard edge. The structure lines of a curved
part are exactly the tangent joints, so this keeps them honest. A swept
strip's facets claim no surface, so its rail is judged against the strip's
own turn across its section: a facet whose normal parts from the side
face's by no more than it parts from the next strip facet in from the rail
reads as a tangent joint, one 60° or more off reads as the strip's end, a
hard edge, and anything in between fails. The next facet in is the one
across an edge that touches neither end of the rail; a triangle on the
rail has none, so its turn is read one facet further, through the strip
facet beside it that has no rail of its own. Where there are several, the
smallest turn counts, so a doubtful rail fails rather than passes. A rail
with nothing to judge it by fails too: a soft rail, a facet with no strip
facet in from it, or one that turns 60° or more to it. A rail more than
60° off its tangent passes as an end.

Which refusals name a cause is decided in one place, `class_of_refusal` in
`crates/api/tests/curves_scoreboard.rs`. A refusal it does not list is
`generic`, so a new one that names its cause is added there, in the same
change as the refusal.

The table is checked in. The gate runs the same cases in the preview build
and fails when a fresh run differs from the table, so a change in any case
shows up as a changed line of `SCOREBOARD.tsv` in the pull request. A case
that got worse does not merge without the maintainer's decision.

**Adding a row.** A model is one JSON file in `corpus/curves/`, named for
what it is. `box.json` is the one to copy.

- `title` and `from`: what the model is, and why it is in the set (what it
  tests, or the tutorial, defect or part it came from).
- `build`: the API calls that make the model, in order, each
  `{"method", "params", "as"?}`. `as` labels a call's result, and a later
  call names a value in it as `{"$ref": "<label>#<JSON pointer>"}`, the
  form `hew.doc.transact` uses. A build call that is refused fails the
  run: the model is wrong, not the engine.
- Lengths. A string made of a number and a unit (`"3mm"`, `"2.5cm"`,
  `"0.1m"`, `"1/8in"`, `"-2mm"`) is read as that length wherever it
  appears in `params`. It starts with a digit or a minus sign. A bare
  number is meters, as in the API.
- `cases`: each has an `id`, the swept call (`method`, and `params` with
  the edges named by locator), and `sizes`, a list of lengths with units.
  The sweep sets `distance` for `hew.solid.chamfer` and `radius` for any
  other call; `sweep` names another parameter. `note` says what the case
  is for.
- `before` lists calls made first, at fixed sizes, for a fillet that meets
  an earlier one. If one of them is refused, the case reports that refusal
  at every size, marked `before[n]`.
- A locator that finds no edge, or more than one, and a name that does not
  exist (an object, a profile) fail the whole run, in the swept call and
  in `before`: the row is wrong.
- Run a sweep from very small to past what fits, so the table shows where
  the answer changes.
- State what you know about the shape, and only what you have worked out.
  All three are optional. `fits_to` is a length: every size up to it
  should build, and a refusal at or below it is counted as open work in
  the table's summary (it does not fail the gate). `never_above` is a
  length past which nothing can exist on the shape: a named limit there
  counts as the right answer, and a built result there is `wrong`.
  `volume` is `"less"` or `"more"`, the way the volume has to move; give
  it only when every edge of the case is an outside corner (less) or an
  inside corner (more).
- Better and worse are judged with these statements counted in, so
  changing a `fits_to` or `never_above` can move a case in the comparison
  with no change in the engine.

Never shape a row to get a better class. A refusal or a wrong result on a
real model is what the table is for. Run `scripts/scoreboard.sh`, read the
comparison, and commit the row and the table together.

## 8. Dependencies & toolchains

Everything Hew builds with is pinned to an exact version, and a bot proposes
each change to those pins as its own pull request.

The pinning is not fussiness. `rust-toolchain.toml` used to say
`channel = "stable"`, which resolves to a different compiler depending on when
and where you run it — and a Clippy lint added in a newer stable failed CI on
a change that was clean on the machine that wrote it. There was no local
command that could have caught it. Meanwhile the actual libraries were already
frozen: both `Cargo.lock` and `pnpm-lock.yaml` are committed and CI installs
with `--frozen-lockfile`, so nothing moved unless someone moved it by hand,
which nobody did. Floating toolchains and frozen libraries is exactly backwards.

**Where each pin lives.** Change any of these by editing the file; there is no
second place to keep in sync.

| Pin | File |
|---|---|
| Rust toolchain | `rust-toolchain.toml` |
| Node.js | `.node-version` |
| Node.js, Desktop E2E lane only | `NODE_VERSION` in `.github/workflows/desktop-e2e.yml` |
| pnpm | `packageManager` in the root `package.json` |
| wasm-pack | `scripts/install-wasm-pack.sh` |
| tauri-driver | `.github/workflows/desktop-e2e.yml` |
| binaryen (wasm-opt, Windows-on-ARM only) | `.github/workflows/release.yml` |
| GitHub Actions | commit SHAs in `.github/workflows/*.yml` |
| Rust crates | `Cargo.lock`, `shells/tauri/src-tauri/Cargo.lock` |
| npm packages | `pnpm-lock.yaml` |

Actions are pinned to a commit SHA rather than a tag, with the version in a
trailing comment. A tag is mutable: `@v4` is whatever its owner last pointed
it at, which is a supply-chain hole in a workflow that holds release signing
keys.

Two deliberate exceptions exist to "one version of a thing", both forced by an
upstream constraint rather than chosen:

- **The Desktop E2E workflow runs an older Node** than the rest of the
  repository, because WebdriverIO calls its own bundled undici with objects
  from Node's built-in one and the two have diverged.
- **`site/` builds on TypeScript 6** where `app/` and `shells/tauri` use 7,
  because `@astrojs/check` accepts only `^5 || ^6`.

Each is pinned where it applies, with the reason and the condition for removing
it written beside the pin. Nothing else should acquire a second version of a
shared tool without the same treatment.

One dependency is deliberately held back rather than diverged: **`opentype.js`
stays on the exact version `1.3.4`** — note the absent caret. Version 2 throws
`substitutionType : 61 lookupType: 6 - substFormat: 1 is not yet supported`
out of `font.getPaths()` on Space Mono, a font this repository ships, which
takes out the whole 3D Text path. It also moves the `name` table under a
platform key while DefinitelyTyped still publishes only v1 types, so
`font.names.fontFamily` keeps typechecking clean and silently returns
`undefined`. Revisit when upstream ships v2 typings and a `getPaths` fix.

**How they get bumped.** [Renovate](https://docs.renovatebot.com) is
configured in `.github/renovate.json5`; the file is commented in full, so read
it rather than this section for the specifics. The shape:

- Patch and minor updates arrive weekly, grouped into one PR per ecosystem,
  and merge themselves once CI is green. Majors come one at a time and wait
  for a person.
- Security advisories ignore the schedule and open immediately.
- Two groups never automerge, because a green CI run is not evidence they are
  good: **Tauri**, and the **Desktop E2E lane's own Node pin**. That workflow
  runs only on push to `main`, never on a pull request, so no PR can exercise
  what either of them affects. Run it by hand after one lands.
- **three.js and Playwright do automerge**, which they did not at first. They
  were held back while the visual-goldens lane was advisory; now that the lane
  blocks, a renderer regression fails the PR instead of slipping past it. A
  Playwright bump can still move the goldens legitimately, by bringing a new
  Chromium — that turns its PR red, and the answer is to run Regen Visual
  Goldens against that branch and commit the PNGs, not to change any code.

That exclusion list is the honest limit of the automation, and it shrinks as
the gate covers more: two of the four original exclusions existed only because
a lane could not fail the build. Everything not on it is safe exactly to the
degree the blocking gate is, which is the argument for keeping the gate honest.

**Whether a dependency is admissible at all** is a separate question from
whether it is current, and `deny.toml` answers it: an advisory database check,
a license allow-list, and a crates.io-only source restriction, run daily over
both cargo workspaces by `.github/workflows/audit.yml` alongside `pnpm audit`.
The source restriction is the mechanical half of rule 7 — it makes SDK-derived
code unable to arrive as a git dependency without a reviewed edit to that file.
It cannot judge provenance, which is what rule 7 is actually about; that still
takes a person. Run it yourself with:

```sh
cargo deny check
cargo deny --manifest-path shells/tauri/src-tauri/Cargo.toml --config deny.toml check
pnpm audit --prod
```

