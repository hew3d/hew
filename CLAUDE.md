# Hew — Agent Instructions

Hew is a cross-platform 3D modeler: SketchUp's interaction model (draw on
faces, push/pull, pervasive inference snapping) on a solids-first data
model where every closed extrusion is a discrete, watertight Object and
combining Objects is always explicit.

Start with these documents — they are the source of truth:

- `docs/DEVELOPMENT.md` — setup, commands, repository layout, and the
  **non-negotiable rules**. Code comments across the kernel cite these as
  "DEVELOPMENT.md rule N". Read them before changing anything.
- `docs/ARCHITECTURE.md` — the data model and crate topology.
- `docs/HEW_FILE_FORMAT.md` — the native format spec; it must be updated
  in the same commit as any serialization change.
- `docs/ROADMAP.md` — what exists and what's planned.

## Quick reference

- Build all crates: `cargo build --workspace`
- Verify (run before every commit): `scripts/verify.sh`
- WASM build: `wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg`
- UI dev server: `pnpm --dir app dev`
- Desktop shell dev: `pnpm --dir shells/tauri dev`

## Ground rules for agents

- The stack (Rust kernel → WASM, TypeScript/React UI, three.js on WebGL2,
  Tauri desktop shell) is settled. Don't relitigate it without explicit
  maintainer approval.
- Kernel crates (`kernel`, `inference`, `tessellate`) stay free of UI,
  I/O, and network dependencies; the WASM boundary lives only in
  `crates/wasm-api`.
- Kernel work is spec-first: signatures + executable specs + property
  tests land with the implementation. Never weaken or delete a failing
  test to get green.
- No silent geometry repair — invalid topology fails with a typed error.
- Ask before: adding a dependency to kernel crates, changing the file
  format, changing the public wasm-api surface, or cross-crate refactors.
- The licensing wall is absolute: nothing derived from the SketchUp SDK
  enters this repo or its dependency chain (see CONTRIBUTING.md).
