# Replay Runner

Headless Node consumer of Hew's record/replay surface
(`docs/DIAGNOSTICS.md`, decision). Loads the kernel's `wasm-api`
compiled for Node, replays committed recording fixtures into a fresh `Scene`,
and asserts the resulting `state_hash` matches each fixture's golden — the
regression guarantee for `crates/wasm-api/src/recording.rs`.

Pure Node, zero npm dependencies.

## 1. Build the wasm bindings

```sh
npm run build
# or directly:
wasm-pack build ../../crates/wasm-api --target nodejs --out-dir ../../tools/replay-runner/pkg-node
```

This writes `pkg-node/` (gitignored — a build artifact, never committed,
mirroring `app/src/wasm/pkg/`). The `--target nodejs` output is CommonJS; this
package is ESM (`"type": "module"`), so `lib.mjs` bridges with
`createRequire`.

## 2. Generate fixtures

```sh
npm run generate
```

Drives a fresh `Scene` through each named scenario in `generate.mjs`
(`start_recording()` … `stop_recording()` … `take_recording()`), and writes
`fixtures/<scenario>.json`. Current scenarios:

- `two-boxes-union-slice` — two overlapping unit boxes (drawn as 4 ground
  segments each, extruded), one `transform_object`-ed to overlap the other,
  `boolean`-unioned, then the union `slice_object`-ed in half. Exercises every
  recorded method except `delete_node`.
- `single-box-delete` — one extruded box, transformed, then removed via
  `delete_node` (the one method the boxes scenario doesn't touch).
- `two-boxes-subtract` / `two-boxes-intersect` — the same overlapping pair
  combined with boolean op 1 (a−b) and op 2 (intersect). The union scenario only
  pins op 0; these lock the other two CSG ops, whose result topology differs.
- `rotated-box` — one box `transform_object`-ed by a 45°-about-Z rotation (+
  translate). The only fixture that drives the rotational affine path (the rest
  only translate); 45° also pins float determinism in the transform.
- `multi-object-scene` — three independent boxes at distinct positions, never
  combined. Locks the multi-object *document* hash (every other fixture
  collapses to one object).
- `chained-boolean` — union a∪b, then subtract c from the result: a boolean
  whose operand is itself a boolean result (handle reuse across a replacing op).
- `hex-prism` — a 6-gon ground profile (the Circle tool's N-gon shape) extruded,
  exercising extrude over a non-quad loop.

To regenerate a single scenario: `node generate.mjs <name>` (or
`node generate.mjs` with no arg to run every scenario, same as
`npm run generate`).

Fixtures are committed as real files in `fixtures/` — they ARE the regression
tests. Regenerate (and re-commit) a fixture only when an intentional kernel
change legitimately moves its golden hash; otherwise a changed golden on
replay is a regression to investigate, not to silently re-bless.

## 3. Run the gate

```sh
npm test
# or: npm start / node run.mjs
```

Loads every `fixtures/*.json`, replays each into `new Scene()`, and prints
`PASS`/`FAIL` per fixture with the hashes:

```
PASS  single-box-delete.json  state_hash=16467438687451334003
PASS  two-boxes-union-slice.json  state_hash=16685420354669910861
```

Exit code is `0` only if every fixture passes; nonzero (with at least one
`FAIL` line) otherwise — e.g. a corrupted `golden_hash`, a malformed
recording, or a call that fails to apply during replay (surfaced by the wasm
side as a thrown `"REPLAY: …"` string).

## Bug → test pipeline (freeze)

The mechanical version of `docs/DEVELOPMENT.md`'s loop: *reported bug →
reproduce from its captured recording → fix → freeze `{recording + golden}`
as a committed fixture → CI replays it forever.*

A "captured reproducer" is a `Recording` JSON — exactly what
`Scene.take_recording()` emits (and what a future bug-report bundle, M17,
will contain). It does not need to already live in `fixtures/`; it can be any
file handed to you alongside a bug report.

```sh
node run.mjs --freeze <recording.json> <fixture-name> [--force]
# or: npm run freeze -- <recording.json> <fixture-name> [--force]
# or directly: node freeze.mjs <recording.json> <fixture-name> [--force]
```

`freeze`:

1. Reads the recording JSON at `<recording.json>`.
2. Validates its shape: `version` must equal the current
   `RECORDING_FORMAT_VERSION` (2 — `docs/DIAGNOSTICS.md`), and it must
   have a `calls` array and a `golden_hash`.
3. Replays it into a fresh `Scene` and compares the resulting `state_hash` to
   the recording's own declared `golden_hash`.
   - **Mismatch → refuses, nonzero exit, no file written.** A reproducer that
     doesn't replay to its own golden is non-deterministic or was captured
     against different kernel code — freezing it would commit a fixture that
     can never pass. This refusal is the entire point of the validation: it's
     the gate that keeps untrustworthy fixtures out of `fixtures/`.
   - Match → proceeds.
4. Sanitizes `<fixture-name>` (basename only, no path separators/traversal;
   `.json` appended if not already present) and refuses to overwrite an
   existing `fixtures/<fixture-name>.json` unless `--force` is passed.
5. Writes the recording **verbatim** — the exact bytes read, never
   re-serialized through `JSON.parse`/`JSON.stringify` (same `golden_hash`
   u64-precision concern as above) — to `fixtures/<fixture-name>.json`.

Once frozen, the fixture is indistinguishable from one written by
`npm run generate`: the next `npm test` / `node run.mjs` (and CI, once
wires this package in) replays it and asserts the golden forever.

## Notes for fixture authors

- `golden_hash` is a `u64`. JS's `JSON.parse` silently loses precision above
  `Number.MAX_SAFE_INTEGER` (e.g. `16685420354669910861` becomes
  `16685420354669910016` as an ordinary `number`), so both `run.mjs` and
  `generate.mjs` extract/write the golden's literal digits via regex rather
  than round-tripping fixture JSON through `JSON.parse`/`JSON.stringify`.
  `replay(json)` itself is unaffected — the wasm side parses the original
  string with `serde_json`, not through any JS JSON path.
- `state_hash()` / `replay()`'s return value marshal as JS `BigInt`
  (wasm-bindgen's mapping for Rust `u64`); fixture comparison uses `BigInt`
  equality (`===`), never `Number`.

## CI wiring — NOT done here

This package does not touch any CI/GitHub config — that is **'s job**
(the CI front, per `docs/ROADMAP.md`). When wiring it in, the pipeline
needs:

1. The wasm-pack nodejs build (`npm run build` in this directory, or the
   equivalent `wasm-pack build` invocation above).
2. `node tools/replay-runner/run.mjs` (or `npm test` from this directory) as
   the regression gate step, failing the job on nonzero exit.

No other setup is required — no npm install (zero dependencies), no browser,
no display server.
