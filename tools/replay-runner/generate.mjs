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

/**
 * Draws a closed regular `n`-gon on a fresh ground sketch, centered at
 * (cx, cy) with circumradius `r`, and returns `[sketchHandle, regionHandle]`.
 * This is the shape the Circle tool emits ( — circles are N-gon profiles),
 * so a fixture built from it exercises the many-edge extrude path the two unit
 * squares never reach.
 */
function groundRegularPolygon(scene, n, cx, cy, r) {
  const sketch = scene.begin_ground_sketch();
  const pt = (i) => {
    const t = (2 * Math.PI * i) / n;
    return [cx + r * Math.cos(t), cy + r * Math.sin(t)];
  };
  let region = null;
  for (let i = 0; i < n; i++) {
    const [ax, ay] = pt(i);
    const [bx, by] = pt((i + 1) % n);
    const report = scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0);
    const created = report.regions_created();
    if (created.length > 0) {
      region = created[0];
    }
  }
  if (region === null) {
    throw new Error(`closing the ${n}-gon did not create a region`);
  }
  return [sketch, region];
}

/** Translation affine (row-major 3x4) by (tx, ty, tz). */
function translate(tx, ty, tz) {
  return [1, 0, 0, tx, 0, 1, 0, ty, 0, 0, 1, tz];
}

/**
 * Two overlapping boxes combined with boolean op `op` (1 = subtract a−b,
 * 2 = intersect). The representative fixture only exercises union (op 0); these
 * lock the goldens for the other two CSG ops, whose result topology is wholly
 * different (subtract carves a recess, intersect keeps only the overlap).
 */
function buildBooleanOp(op) {
  return (scene) => {
    scene.start_recording();
    const [s1, r1] = groundUnitSquare(scene);
    const a = scene.extrude_region(s1, r1, 2.0);
    const [s2, r2] = groundUnitSquare(scene);
    const b = scene.extrude_region(s2, r2, 1.0);
    scene.transform_object(b, translate(0.5, 0.5, 0));
    scene.boolean(op, a, b);
    scene.stop_recording();
    return scene.take_recording();
  };
}

/**
 * A single box rotated 45° about Z (then nudged) — the existing fixtures only
 * ever translate, so this is the only one that drives the rotational part of
 * the affine multiply through replay. 45° gives irrational f64 vertex coords,
 * so it also pins float determinism in `transform_object`.
 */
function buildRotatedBox(scene) {
  scene.start_recording();
  const [sketch, region] = groundUnitSquare(scene);
  const obj = scene.extrude_region(sketch, region, 1.5);
  const c = Math.SQRT1_2; // cos 45° = sin 45°
  // Row-major 3x4: rotate 45° about +Z, translate +2 in x.
  scene.transform_object(obj, [c, -c, 0, 2, c, c, 0, 0, 0, 0, 1, 0]);
  scene.stop_recording();
  return scene.take_recording();
}

/**
 * Three independent boxes at distinct positions, never combined — locks the
 * multi-object *document* hash (vs. every other fixture, which collapses to one
 * object). Mirrors the multi-object visual scene at the kernel layer.
 */
function buildMultiObjectScene(scene) {
  scene.start_recording();
  const [s1, r1] = groundUnitSquare(scene);
  scene.extrude_region(s1, r1, 1.0);
  const [s2, r2] = groundUnitSquare(scene);
  const b = scene.extrude_region(s2, r2, 2.0);
  scene.transform_object(b, translate(3, 0, 0));
  const [s3, r3] = groundUnitSquare(scene);
  const c = scene.extrude_region(s3, r3, 0.5);
  scene.transform_object(c, translate(0, 3, 0));
  scene.stop_recording();
  return scene.take_recording();
}

/**
 * Chained CSG: union a∪b, then subtract c from the result. Exercises a boolean
 * whose operand is itself a boolean result (handle reuse across a replacing op
 * — the path that surfaced the tree-consistency bug).
 */
function buildChainedBoolean(scene) {
  scene.start_recording();
  const [s1, r1] = groundUnitSquare(scene);
  const a = scene.extrude_region(s1, r1, 2.0);
  const [s2, r2] = groundUnitSquare(scene);
  const b = scene.extrude_region(s2, r2, 2.0);
  scene.transform_object(b, translate(0.5, 0.5, 0));
  const ab = scene.boolean(0 /* union */, a, b);
  const [s3, r3] = groundUnitSquare(scene);
  const c = scene.extrude_region(s3, r3, 1.0);
  scene.transform_object(c, translate(0.25, 0.25, 0.5));
  scene.boolean(1 /* subtract */, ab, c);
  scene.stop_recording();
  return scene.take_recording();
}

/**
 * A hexagonal prism: a 6-gon ground profile extruded. The many-edge profile
 * (the Circle tool's output shape) drives extrude over a non-quad loop.
 */
function buildHexPrism(scene) {
  scene.start_recording();
  const [sketch, region] = groundRegularPolygon(scene, 6, 0, 0, 1);
  scene.extrude_region(sketch, region, 1.0);
  scene.stop_recording();
  return scene.take_recording();
}

/** Named scenarios, each producing a `take_recording()` JSON string. */
const SCENARIOS = {
  'two-boxes-union-slice': buildTwoBoxesUnionSlice,
  'single-box-delete': buildSingleBoxDelete,
  'two-boxes-subtract': buildBooleanOp(1),
  'two-boxes-intersect': buildBooleanOp(2),
  'rotated-box': buildRotatedBox,
  'multi-object-scene': buildMultiObjectScene,
  'chained-boolean': buildChainedBoolean,
  'hex-prism': buildHexPrism,
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
