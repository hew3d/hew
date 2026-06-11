# Developing Hew

This guide covers environment setup, repository layout, and the
engineering rules every change is held to.

## 1. Prerequisites & setup

- **Rust**, stable channel, pinned by `rust-toolchain.toml` (with `clippy`,
  `rustfmt`, and the `wasm32-unknown-unknown` target). With `rustup`, any
  `cargo` command from the repository root installs the right toolchain.
- **`wasm-pack`**, to compile the kernel to WebAssembly.
- **Node.js** and **pnpm** for the TypeScript app.
- **Tauri's platform prerequisites**, if you're building the desktop shell.

```sh
cargo build --workspace          # build every Rust crate
pnpm install                     # install JS/TS dependencies
wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg
pnpm --dir app dev               # UI dev server (web build)
pnpm --dir shells/tauri dev      # desktop shell dev
```

The WASM build step is required before the UI dev server will run — the
app imports the kernel bindings from `app/src/wasm/pkg`, which is
generated, not checked in.

Before sending a change, run the full verification gate:

```sh
scripts/verify.sh
```

## 2. Repository layout

```
crates/kernel/       half-edge mesh, Objects, sticky-geometry rules,
                     watertightness tracking, undo, serialization
crates/inference/    snapping/inference engine (pure geometry queries)
crates/tessellate/   kernel topology -> render buffers
crates/wasm-api/     wasm-bindgen surface exposing the kernel to the UI
app/                 TypeScript UI (viewport, tools, panels)
shells/tauri/        desktop shell
shells/web/          static web build
scripts/             verify.sh, the pre-merge verification gate
```

Interop crates (foreign-format importers) follow one shape when they
exist: parse a foreign format into Hew's Objects, depend on `kernel`,
never the reverse.

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
   validator coverage in the same change.
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
   test is the rule, full stop. `.skp` support, if it ever exists, comes
   only through a reader whose entire provenance is demonstrably clean.
8. **Discuss before.** Adding a new dependency to a kernel crate, changing
   the native file format, changing the public `wasm-api` surface, or any
   refactor that crosses crate boundaries is proposed and discussed before
   it lands — never landed silently inside an unrelated change.

## 4. Kernel development

Kernel work is spec-first. The contract for an operation — a doc comment
describing behavior, typed error cases, tolerance obligations, and
transactionality — lands with executable specs in
`crates/kernel/tests/op_specs.rs` before or alongside the implementation.
Implementing an operation means making an existing contract true, not
designing one on the fly. Every public mutation ends with the debug-build
invariant validator; mutating operations carry the strong exception
guarantee (on `Err`, the Object is untouched).

## 5. Verification

`scripts/verify.sh` is the single gate a change must pass before it
merges: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace`, the WASM build, and the app's
type-check/test/build steps.
