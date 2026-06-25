#!/usr/bin/env node
// Generate mode for the replay runner: drives a fresh Scene through a
// representative model, records the committed call stream, and writes the
// result as a fixture JSON (docs/DIAGNOSTICS.md) under fixtures/.
//
// Can be run directly (`node generate.mjs [name]`) or via `node run.mjs
// --generate`, which dynamically imports `generate()` below.
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { loadWasm, FIXTURES_DIR } from './lib.mjs';

/**
 * Draws a closed unit square on a fresh ground sketch (4 segments forming a
 * loop) and returns `[sketchHandle, regionHandle]` — the region handle comes
 * from the closing segment's `regions_created()` (mirrors the kernel test
 * helper `ground_unit_square` in crates/wasm-api/src/lib.rs).
 */
function groundUnitSquare(scene) {
  const sketch = scene.begin_ground_sketch();
  const corners = [
    [0, 0, 1, 0],
    [1, 0, 1, 1],
    [1, 1, 0, 1],
    [0, 1, 0, 0],
  ];
  let region = null;
  for (const [ax, ay, bx, by] of corners) {
    const report = scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0);
    const created = report.regions_created();
    if (created.length > 0) {
      region = created[0];
    }
  }
  if (region === null) {
    throw new Error('closing the square did not create a region');
  }
  return [sketch, region];
}

/**
 * Builds the representative fixture model: two overlapping unit boxes
 * (drawn as 4 ground segments each, then extruded), one transformed to
 * overlap the other, unioned, then the union sliced in half. Exercises every
 * recorded method in docs/DIAGNOSTICS.md except delete_node.
 */
function buildTwoBoxesUnionSlice(scene) {
  scene.start_recording();

  const [s1, r1] = groundUnitSquare(scene);
  const a = scene.extrude_region(s1, r1, 2.0);

  const [s2, r2] = groundUnitSquare(scene);
  const b = scene.extrude_region(s2, r2, 1.0);

  // Shift b by (0.5, 0.5, 0) so it overlaps a.
  // Row-major 3x4 affine: [m00 m01 m02 tx, m10 m11 m12 ty, m20 m21 m22 tz].
  scene.transform_object(b, [1, 0, 0, 0.5, 0, 1, 0, 0.5, 0, 0, 1, 0]);

  const union = scene.boolean(0 /* union */, a, b);

  // Slice the union in half through its midplane (z=1, normal +z).
  scene.slice_object(union, [0, 0, 1, 0, 0, 1]);

  scene.stop_recording();
  return scene.take_recording();
}

/**
 * Builds a smaller fixture exercising a different shape of the surface than
 * the boxes scenario: a single extruded box, transformed, then removed via
 * `delete_node` — the one recorded method the boxes scenario doesn't touch.
 */
function buildSingleBoxDelete(scene) {
  scene.start_recording();

  const [sketch, region] = groundUnitSquare(scene);
  const obj = scene.extrude_region(sketch, region, 3.0);
  scene.transform_object(obj, [1, 0, 0, 2, 0, 1, 0, 0, 0, 0, 1, 0]);
  scene.delete_node(0 /* object */, obj);

  scene.stop_recording();
  return scene.take_recording();
}

/** Named scenarios, each producing a `take_recording()` JSON string. */
const SCENARIOS = {
  'two-boxes-union-slice': buildTwoBoxesUnionSlice,
  'single-box-delete': buildSingleBoxDelete,
};

/** Runs scenario `name`'s build and writes `fixtures/<name>.json`. */
export async function generate(name = 'two-boxes-union-slice') {
  const build = SCENARIOS[name];
  if (!build) {
    throw new Error(
      `unknown scenario "${name}" — known: ${Object.keys(SCENARIOS).join(', ')}`,
    );
  }

  const wasm = loadWasm();
  const scene = new wasm.Scene();
  const json = build(scene);
  scene.free?.();

  await mkdir(FIXTURES_DIR, { recursive: true });
  const outPath = path.join(FIXTURES_DIR, `${name}.json`);
  // Write take_recording()'s JSON verbatim — NOT round-tripped through
  // JSON.parse/JSON.stringify. golden_hash is a u64 (docs/DIAGNOSTICS.md)
  // and JS's JSON.parse silently loses precision above
  // Number.MAX_SAFE_INTEGER (e.g. 16685420354669910861 becomes
  // 16685420354669910016), which would bake a corrupted golden into the
  // fixture file. take_recording() already emits compact, stable JSON, so no
  // reformatting is needed.
  await writeFile(outPath, json + '\n', 'utf8');

  const golden = json.match(/"golden_hash"\s*:\s*(\d+)/)?.[1] ?? '?';
  const callCount = json.match(/"method"/g)?.length ?? 0;
  console.log(`Wrote ${outPath} (${callCount} calls, golden_hash=${golden})`);
  return outPath;
}

/** Runs every named scenario, writing one fixture file each. */
export async function generateAll() {
  for (const name of Object.keys(SCENARIOS)) {
    await generate(name);
  }
}

// Allow `node generate.mjs [name]` directly, in addition to `run.mjs
// --generate`. With no name, generates every scenario.
if (import.meta.url === `file://${process.argv[1]}`) {
  const arg = process.argv[2];
  (arg ? generate(arg) : generateAll()).catch((err) => {
    console.error(err);
    process.exitCode = 1;
  });
}
