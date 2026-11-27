#!/usr/bin/env node
// Generate mode for the replay runner: drives a fresh Scene through a
// representative model, records the committed call stream, and writes the
// result as a fixture JSON (docs/dev/DIAGNOSTICS.md) under fixtures/.
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
 * recorded method in docs/dev/DIAGNOSTICS.md except delete_node.
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

/**
 * Draws a closed regular `n`-gon on a fresh sketch in the plane x = `x`
 * (normal +X), centered at (y = cy, z = cz) with circumradius `r`, and returns
 * `[sketchHandle, regionHandle]` — a disk drawn in the plane of a solid's
 * side face but out in empty space, a cutter built clear of its target.
 */
function sidePlaneRegularPolygon(scene, n, x, cy, cz, r) {
  const sketch = scene.begin_sketch_on_plane(x, cy, cz, 1, 0, 0);
  const pt = (i) => {
    const t = (2 * Math.PI * i) / n;
    return [cy + r * Math.cos(t), cz + r * Math.sin(t)];
  };
  let region = null;
  for (let i = 0; i < n; i++) {
    const [ay, az] = pt(i);
    const [by, bz] = pt((i + 1) % n);
    const report = scene.sketch_add_segment(sketch, x, ay, az, x, by, bz);
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

/**
 * A cutter built clear of its target, in meters: a 7 × 5 × 6 cm bin, a 2 cm
 * disk drawn in the plane of its right face (x = 0.07) 3 cm above the top
 * corner at y = 0 (the upper-left corner as the Right view shows it, eye at
 * +X looking toward -X with Z up, so screen-left is y = 0), pushed 7 cm
 * across the bin into a cylinder that hangs just above it, then moved down
 * 3 cm so its axis runs along the top edge, and subtracted. Before the move
 * the cutter only hangs over the bin, so that
 * Subtract is refused (`NothingRemoved`) and leaves nothing in the
 * recording — the scenario asserts the refusal rather than recording it —
 * and the fixture locks the cut that the moved cutter makes.
 */
function buildCutterMovedIntoBin(scene) {
  scene.start_recording();

  const bin = scene.begin_ground_sketch();
  const footprint = [
    [0, 0, 0.07, 0],
    [0.07, 0, 0.07, 0.05],
    [0.07, 0.05, 0, 0.05],
    [0, 0.05, 0, 0],
  ];
  let binRegion = null;
  for (const [ax, ay, bx, by] of footprint) {
    const created = scene.sketch_add_segment(bin, ax, ay, 0, bx, by, 0).regions_created();
    if (created.length > 0) binRegion = created[0];
  }
  if (binRegion === null) throw new Error('closing the bin footprint did not create a region');
  const block = scene.extrude_region(bin, binRegion, 0.06);

  const [disk, diskRegion] = sidePlaneRegularPolygon(scene, 24, 0.07, 0.0, 0.09, 0.02);
  const cutter = scene.extrude_region(disk, diskRegion, -0.07);

  // Hanging a centimeter above the bin: nothing to remove, refused, not
  // recorded (refused calls never enter a recording).
  let refused = null;
  try {
    scene.boolean_nodes(1 /* subtract */, 0, block, 0, cutter);
  } catch (err) {
    refused = String(err);
  }
  if (refused === null || !refused.startsWith('NothingRemoved:')) {
    throw new Error(`expected the hanging cutter's subtract to refuse NothingRemoved, got ${refused}`);
  }

  scene.transform_object(cutter, translate(0, 0, -0.03));
  scene.boolean_nodes(1 /* subtract */, 0, block, 0, cutter);

  scene.stop_recording();
  return scene.take_recording();
}

/**
 * A 24-gon cup: a prism shelled 0.05 inward with its top (picked by a ray
 * straight down) left open. Locks Shell's recorded spec and its result.
 */
function buildShellCup(scene) {
  scene.start_recording();
  const [sketch, region] = groundRegularPolygon(scene, 24, 0, 0, 0.5);
  const cup = scene.extrude_region(sketch, region, 1.0);
  const top = scene.pick_face(0, 0, 5, 0, 0, -1);
  if (!top) {
    throw new Error('the ray straight down misses the prism top');
  }
  // Face ids are u64 (BigInt in JS), which JSON.stringify refuses.
  scene.shell(cup, `{"open":[${top.face()}],"thickness":0.05}`);
  scene.stop_recording();
  return scene.take_recording();
}
// ==== true-curves: shapes ====
/**
 * Draws a goblet half-profile in the XZ plane with one side on the Z axis —
 * a foot, a stem, and a bowl whose outer wall is a facetted arc carrying its
 * exact circle — and revolves it a full turn about Z: one closed solid whose
 * walls are planes, cylinders, a cone and a sphere band.
 */
function buildRevolveGoblet(scene) {
  scene.start_recording();
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, -1, 0);
  // Bowl: circle centered (0, 0.8) radius 0.3, from the stem top (0.1, z0)
  // up to the rim (0.3, 0.8), facetted in 6 steps.
  const r = 0.3;
  const z0 = 0.8 - Math.sqrt(r * r - 0.1 * 0.1);
  const a0 = Math.atan2(z0 - 0.8, 0.1);
  const bowl = [];
  for (let i = 0; i <= 6; i++) {
    const a = a0 + ((0 - a0) * i) / 6;
    bowl.push([r * Math.cos(a), 0.8 + r * Math.sin(a)]);
  }
  bowl[0] = [0.1, z0];
  bowl[6] = [0.3, 0.8];
  const seg = (a, b) => scene.sketch_add_segment(sketch, a[0], 0, a[1], b[0], 0, b[1]);
  const regions = [];
  const keep = (report) => regions.push(...report.regions_created());
  keep(seg([0, 0], [0.3, 0]));
  keep(seg([0.3, 0], [0.3, 0.04]));
  keep(seg([0.3, 0.04], [0.1, 0.12]));
  keep(seg([0.1, 0.12], bowl[0]));
  scene.sketch_begin_curve_with(sketch, 0, 0, 0.8, r);
  for (let i = 0; i < 6; i++) keep(seg(bowl[i], bowl[i + 1]));
  scene.sketch_end_curve(sketch);
  keep(seg([0.3, 0.8], [0, 0.8]));
  keep(seg([0, 0.8], [0, 0]));
  if (regions.length !== 1) {
    throw new Error(`goblet profile made ${regions.length} regions`);
  }
  const profile = JSON.stringify({
    region: { sketch: String(sketch), region: String(regions[0]) },
  });
  const spec = JSON.stringify({
    axis_point: [0, 0, 0],
    axis: [0, 0, 1],
    angle: 2 * Math.PI,
    segments: 24,
  });
  scene.revolve(profile, spec);
  scene.stop_recording();
  return scene.take_recording();
}

/**
 * The same cutter drawn the way the Getting Started guide now draws it: the
 * bin as above, then a 2 cm disk centered on the right face's upper corner
 * at y = 0 drawn as a DETACHED shape (`draft_begin_detached` on the face),
 * so the face's edge does not split it and the face stays whole; the disk is
 * pushed 7 cm across the bin into a cylinder whose axis runs along the top
 * edge, and subtracted. The fixture locks a recording holding
 * `draft_begin_detached` and the cut it makes.
 */
function buildDetachedCutterOnBin(scene) {
  scene.start_recording();

  const bin = scene.begin_ground_sketch();
  const footprint = [
    [0, 0, 0.07, 0],
    [0.07, 0, 0.07, 0.05],
    [0.07, 0.05, 0, 0.05],
    [0, 0.05, 0, 0],
  ];
  let binRegion = null;
  for (const [ax, ay, bx, by] of footprint) {
    const created = scene.sketch_add_segment(bin, ax, ay, 0, bx, by, 0).regions_created();
    if (created.length > 0) binRegion = created[0];
  }
  if (binRegion === null) throw new Error('closing the bin footprint did not create a region');
  const block = scene.extrude_region(bin, binRegion, 0.06);

  // The right face (x = 0.07, normal +X), found by a ray from +X.
  const face = scene.pick_face_geometry(1, 0.025, 0.03, -1, 0, 0);
  if (!face) throw new Error("the ray missed the bin's right face");
  // Ids are u64 (BigInt here): written into the JSON by their digits.
  const ctx = `{"face":{"object":${face.object()},"face":${face.face()}}}`;

  const [cy, cz, r, n] = [0.0, 0.06, 0.02, 48];
  const d = scene.draft_begin_detached(ctx, 'Circle');
  scene.draft_begin_curve_with(d, [0.07, cy, cz], r);
  const at = (i) => {
    const t = (2 * Math.PI * i) / n;
    return [cy + r * Math.cos(t), cz + r * Math.sin(t)];
  };
  for (let i = 0; i < n; i++) {
    const [ay, az] = at(i);
    const [by, bz] = at((i + 1) % n);
    scene.draft_add_segment(d, 0.07, ay, az, 0.07, by, bz);
  }
  scene.draft_end_curve(d);
  scene.draft_end(d);

  // The whole disk is one detached region on the face's plane.
  const pick = scene.pick_sketch_region(1, 0.005, 0.055, -1, 0, 0);
  if (!pick || !pick.detached()) throw new Error('expected to pick the detached disk');
  const cutter = scene.extrude_region(pick.sketch(), pick.region(), -0.07);
  scene.boolean_nodes(1 /* subtract */, 0, block, 0, cutter);

  scene.stop_recording();
  return scene.take_recording();
}

/**
 * Sweeps a 0.2 m square along a straight 1 m line with a 90° twist and a
 * 40% end: Follow Me's generated sweep through the skin builder.
 */
function buildFollowMeTwist(scene) {
  scene.start_recording();
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, -1, 0);
  const pts = [[0, 0], [0.2, 0], [0.2, 0.2], [0, 0.2]];
  const regions = [];
  for (let i = 0; i < 4; i++) {
    const [a, b] = [pts[i], pts[(i + 1) % 4]];
    regions.push(...scene.sketch_add_segment(sketch, a[0], 0, a[1], b[0], 0, b[1]).regions_created());
  }
  const path = scene.begin_ground_sketch();
  const edges = scene.sketch_add_segment(path, 0.1, 0, 0, 0.1, -1, 0).new_edges();
  scene.follow_me_along_edges_with(sketch, regions[0], path, edges, undefined, undefined, Math.PI / 2, 0.4, 0);
  scene.stop_recording();
  return scene.take_recording();
}

/**
 * The editing-scope pick (Shaded and X-ray) (docs/design/xray-edit-scope-pick.md), the
 * reporter's gesture: a box A with a second box B half buried in it, a
 * downward ray through A's top that also meets B's buried top. `set_render_mode`
 * is view state and is NOT recorded, so this builder asks every scoped pick
 * itself and asserts the answer at generation. The recording carries the
 * geometry and the `set_pick_scope` / `clear_pick_scope` calls; its replay
 * re-applies the scope. The golden state_hash covers neither the scope nor
 * the render mode, so the replay gate (run.mjs, POST_REPLAY_PROBES) re-asks
 * the same scoped picks after replay; the golden alone pins the document.
 */
function buildXrayEditScope(scene) {
  scene.start_recording();

  const box = (x0, y0, x1, y1, height) => {
    const sketch = scene.begin_ground_sketch();
    const loop = [
      [x0, y0, x1, y0],
      [x1, y0, x1, y1],
      [x1, y1, x0, y1],
      [x0, y1, x0, y0],
    ];
    let region = null;
    for (const [ax, ay, bx, by] of loop) {
      const created = scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0).regions_created();
      if (created.length > 0) region = created[0];
    }
    if (region === null) throw new Error('closing the box footprint did not create a region');
    return scene.extrude_region(sketch, region, height);
  };
  const a = box(0, 0, 2, 2, 2);
  const b = box(1, 0.5, 3, 1.5, 1);

  const pickObject = (x, y) => {
    const hit = scene.pick_face(x, y, 8, 0, 0, -1);
    if (hit === undefined) return null;
    const object = hit.object();
    hit.free();
    return object;
  };
  const expectPick = (label, x, y, want) => {
    const got = pickObject(x, y);
    if (got !== want) throw new Error(`${label}: expected object ${want}, picked ${got}`);
  };

  // Through A's top at (1.5, 1): A is the nearest face, B lies behind it.
  scene.set_render_mode('xray');
  expectPick('X-ray, no scope', 1.5, 1, a);
  scene.set_pick_scope(new BigUint64Array([b]), new BigUint64Array([]), true);
  expectPick('X-ray, B in scope', 1.5, 1, b);
  // Nothing in scope under the ray: today's answer, the nearest face.
  expectPick('X-ray, B in scope, nothing of B behind', 0.4, 1, a);
  // Shaded follows the scope too (faces are targets there).
  scene.set_render_mode('shaded');
  expectPick('Shaded, B in scope', 1.5, 1, b);
  scene.set_render_mode('xray');
  scene.clear_pick_scope();
  expectPick('X-ray, scope cleared', 1.5, 1, a);
  // Leave the scope set at the end of the recording so a replay re-applies it.
  scene.set_pick_scope(new BigUint64Array([b]), new BigUint64Array([]), true);

  scene.stop_recording();
  return scene.take_recording();
}

/**
 * A 0.2 × 0.2 × 1 bar twisted 90° about its long axis, then bent 60%
 * of a quarter turn: two Deform reshapes, each one undo step.
 */
function buildDeformTwistBend(scene) {
  scene.start_recording();
  const [sketch, region] = groundUnitSquare(scene);
  const bar = scene.extrude_region(sketch, region, 1.0);
  scene.transform_object(bar, [0.2, 0, 0, 0, 0, 0.2, 0, 0, 0, 0, 1, 0]);
  const frame = { origin: [0.1, 0.1, 0], axis: [0, 0, 1], u: [1, 0, 0], length: 1 };
  scene.deform(bar, JSON.stringify({ mode: 'twist', frame, angle: Math.PI / 2 }));
  scene.deform(bar, JSON.stringify({ mode: 'bend', frame, angle: 0.6 * (Math.PI / 2) }));
  scene.stop_recording();
  return scene.take_recording();
}
/**
 * Five rectangles on parallel planes, wide in the middle, lofted through a
 * smooth hull (4 stations per span).
 */
function buildLoftHull(scene) {
  scene.start_recording();
  const sizes = [[0.2, 0.1], [0.4, 0.3], [0.5, 0.4], [0.4, 0.3], [0.2, 0.1]];
  const sources = sizes.map(([w, d], i) => {
    const z = i;
    const sketch = scene.begin_sketch_on_plane(0, 0, z, 0, 0, 1);
    const c = [[-w, -d], [w, -d], [w, d], [-w, d]];
    let region = null;
    for (let k = 0; k < 4; k++) {
      const [a, b] = [c[k], c[(k + 1) % 4]];
      const created = scene.sketch_add_segment(sketch, a[0], a[1], z, b[0], b[1], z).regions_created();
      if (created.length > 0) region = created[0];
    }
    return { region: { sketch: String(sketch), region: String(region) } };
  });
  scene.loft(JSON.stringify({ sources, stations: 4 }));
  scene.stop_recording();
  return scene.take_recording();
}
// ==== end true-curves: shapes ====

/**
 * A label pressed into a round prism: a 24-gon prism, a rectangle drawn on
 * a vertical plane in front of it, imprinted along the plane's normal, then
 * debossed 0.01 following the surface. Locks Project's two recorded steps.
 */
function buildProjectLabel(scene) {
  scene.start_recording();
  const [sketch, region] = groundRegularPolygon(scene, 24, 0, 0, 0.5);
  const post = scene.extrude_region(sketch, region, 1.0);
  const label = scene.begin_sketch_on_plane(-2, 0, 0, -1, 0, 0);
  const c = [
    [-0.15, 0.4],
    [0.15, 0.4],
    [0.15, 0.6],
    [-0.15, 0.6],
  ];
  for (let i = 0; i < 4; i++) {
    const [ay, az] = c[i];
    const [by, bz] = c[(i + 1) % 4];
    scene.sketch_add_segment(label, -2, ay, az, -2, by, bz);
  }
  const faces = scene.project_imprint(post, label, new BigUint64Array());
  scene.project_depth(post, label, faces, 0.01, 1 /* follow */);
  scene.stop_recording();
  return scene.take_recording();
}

/**
 * A cushion: a 1 × 1 × 0.3 slab smoothed to level 2, then baked into real
 * faces. Locks Smooth's level write and Bake's subdivided solid.
 */
function buildSmoothCushion(scene) {
  scene.start_recording();
  const [sketch, region] = groundUnitSquare(scene);
  const slab = scene.extrude_region(sketch, region, 0.3);
  scene.set_smooth_level(slab, 2);
  scene.bake(slab);
  scene.stop_recording();
  return scene.take_recording();
}

// ==== true-curves: blend ====
/**
 * A unit box with its top rim filleted (the four top edges, picked through
 * the top face's loop), the fillet resized, removed, then undone and redone.
 * Locks the recorded blend calls and their replay.
 */
function buildFilletBoxRim(scene) {
  scene.start_recording();
  const [sketch, region] = groundUnitSquare(scene);
  const box = scene.extrude_region(sketch, region, 1.0);
  const top = scene.pick_face(0.5, 0.5, 5, 0, 0, -1);
  if (!top) {
    throw new Error('the ray straight down misses the box top');
  }
  const loop = scene.face_loop_edges(box, top.face(), 0.5, 0.02, 1.0);
  const edges = Array.from(loop.edges()).map((e) => `"${e}"`).join(',');
  const round = (r) => `{"kind":"round","radius":${r},"segments":6}`;
  const report = scene.blend_edges(box, `{"edges":[${edges}],"profile":${round(0.1)}}`);
  const [blend] = Array.from(report.blend_ids());
  scene.resize_blend(box, blend, round(0.2));
  const [resized] = JSON.parse(scene.blend_features_json(box));
  scene.remove_blend(box, resized.id);
  scene.scene_undo();
  scene.scene_redo();
  scene.stop_recording();
  return scene.take_recording();
}
// ==== end true-curves: blend ====
/**
 * Array along a path: a box copied along a bent two-segment ground path, six
 * instances with a 100% to 40% scale ramp, copies turned to follow the path.
 * Drives the path frames (a corner's parallel transport), the similarity
 * placements and the single-undo array record.
 */
function buildPathArrayRamp(scene) {
  scene.start_recording();
  const [sketch, region] = groundUnitSquare(scene);
  const box = scene.extrude_region(sketch, region, 0.5);
  const path = scene.begin_ground_sketch();
  const edges = [
    ...scene.sketch_add_segment(path, 0, -3, 0, 3, -3, 0).new_edges(),
    ...scene.sketch_add_segment(path, 3, -3, 0, 3, 0, 0).new_edges(),
  ];
  const pathJson = JSON.stringify({
    kind: 'sketch_edges',
    sketch: path.toString(),
    edges: edges.map((e) => e.toString()),
  });
  scene.array_along_path(
    new Uint8Array([0]),
    new BigUint64Array([box]),
    pathJson,
    6,
    1.0,
    0.4,
    true,
  );
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
  'cutter-moved-into-bin': buildCutterMovedIntoBin,
  'detached-cutter-on-bin': buildDetachedCutterOnBin,
  'xray-edit-scope': buildXrayEditScope,
  'path-array-ramp': buildPathArrayRamp,
  'shell-cup': buildShellCup,
  'project-label': buildProjectLabel,
  'smooth-cushion': buildSmoothCushion,
  // ==== true-curves: blend ====
  'fillet-box-rim': buildFilletBoxRim,
  // ==== end true-curves: blend ====
  // ==== true-curves: shapes ====
  'revolve-goblet': buildRevolveGoblet,
  'follow-me-twist': buildFollowMeTwist,
  'deform-twist-bend': buildDeformTwistBend,
  'loft-hull': buildLoftHull,
  // ==== end true-curves: shapes ====
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
  // JSON.parse/JSON.stringify. golden_hash is a u64 (docs/dev/DIAGNOSTICS.md)
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
