# Hew

A cross-platform 3D modeler that pairs SketchUp-style direct modeling with
a solids-first geometry kernel.

Hew keeps the interaction model that made SketchUp approachable — draw on
faces, push/pull, pervasive inference snapping — and puts it on top of a
stricter foundation:

- Extruding a closed profile creates a discrete **Object**: a watertight
  solid by construction that never welds to another Object implicitly.
- Combining Objects is always explicit — a boolean union or merge you ask
  for, never a side effect of geometry touching.
- Inside an Object, sticky geometry applies: drawing an edge across a face
  splits it, closed coplanar loops become faces, faces push/pull.
- No silent repair: operations that would produce invalid topology fail
  with a typed error.

The geometry kernel is pure Rust compiled to WebAssembly and runs inside
the webview on every platform, desktop included. The UI is a single
TypeScript + React codebase rendering through three.js on a WebGL2
baseline; the desktop shell is Tauri.

Hew is in early development and changes quickly.

## Building from source

Prerequisites: Rust (pinned by `rust-toolchain.toml`), `wasm-pack`,
Node.js with `pnpm`, and the
[Tauri platform prerequisites](https://tauri.app/start/prerequisites/)
for the desktop shell.

```sh
scripts/verify.sh                 # build + test + lint gate
wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg
pnpm --dir app dev                # run the app in a browser
pnpm --dir shells/tauri dev       # run the desktop shell
```

See `docs/DEVELOPMENT.md` for the repository layout and the engineering
rules every change is held to.
