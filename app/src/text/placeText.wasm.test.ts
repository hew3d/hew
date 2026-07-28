/**
 * End-to-end 3D Text placement through the REAL compiled wasm module —
 * the "assert through the kernel API" requirement (docs/design/3d-text.md)
 * for the glyph-with-counter region resolution, exercised from the TS side
 * this time (the Rust-side equivalent lives in
 * `crates/wasm-api/src/lib.rs`'s `place_text_o_glyph_via_full_gesture_pipeline_is_watertight`).
 * Mirrors `wasm/loader.test.ts`'s real-wasm-in-vitest pattern.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { classifyContourFill, type Pt2 } from './flatten'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmPath = resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm')
  const wasmBytes = await readFile(wasmPath)
  await init({ module_or_path: wasmBytes })
})

/** Draws an 'O'-like glyph (outer square + smaller concentric counter
 *  square) on the ground sketch, bracketed as one gesture — the exact
 *  sequence `TextPlaceTool._commit` uses. Returns the sketch handle. */
function drawOGlyph(scene: Scene): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  scene.sketch_begin_gesture(sketch)
  const outer: Array<[number, number]> = [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
  ]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = outer[i]
    const [bx, by] = outer[(i + 1) % 4]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  const inner: Array<[number, number]> = [
    [-0.4, -0.4],
    [0.4, -0.4],
    [0.4, 0.4],
    [-0.4, 0.4],
  ]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = inner[i]
    const [bx, by] = inner[(i + 1) % 4]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  scene.sketch_end_gesture(sketch)
  return sketch
}

/** Picks the fill region(s) among `sketch`'s resolved regions using the
 *  SAME even-odd nesting-depth rule `classifyContourFill` applies to raw
 *  glyph contours — here applied directly to the kernel's own resolved
 *  region boundaries (`region_boundary`), proving the selection algorithm
 *  agrees with the kernel's real region topology, not just a hand-modeled
 *  approximation of it. */
function selectFillRegions(scene: Scene, sketch: bigint): bigint[] {
  const regionIds = Array.from(scene.sketch_regions(sketch))
  const boundaries: Pt2[][] = regionIds.map((r) => {
    const flat = scene.region_boundary(sketch, r)
    const pts: Pt2[] = []
    for (let i = 0; i + 2 < flat.length; i += 3) {
      pts.push([flat[i], flat[i + 1]])
    }
    return pts
  })
  const fills = classifyContourFill(boundaries)
  return regionIds.filter((_, i) => fills[i])
}

describe('place_text through the real wasm module', () => {
  it("an 'O' glyph resolves to one region with one hole, extrudes only that region, and is watertight", () => {
    const scene = new Scene()
    const sketch = drawOGlyph(scene)

    const regionIds = Array.from(scene.sketch_regions(sketch))
    expect(regionIds).toHaveLength(2)

    const fillRegions = selectFillRegions(scene, sketch)
    expect(fillRegions).toHaveLength(1)

    const instance = scene.place_text(
      sketch,
      BigUint64Array.from(fillRegions),
      0.5,
      '3D Text "O"',
      undefined,
    )
    const comp = scene.instance_def(instance)
    expect(comp).toBeDefined()
    const members = scene.component_member_objects(comp as bigint)
    expect(members).toHaveLength(1)
    expect(scene.object_watertight(members[0])).toBe(true)
  })

  it('the whole placement undoes and redoes as a single step', () => {
    const scene = new Scene()
    const sketch = drawOGlyph(scene)
    const fillRegions = selectFillRegions(scene, sketch)

    const instance = scene.place_text(
      sketch,
      BigUint64Array.from(fillRegions),
      0.5,
      '3D Text "O"',
      undefined,
    )
    const comp = scene.instance_def(instance) as bigint

    scene.scene_undo()
    expect(scene.instance_ids()).not.toContain(instance)
    expect(() => scene.sketch_regions(sketch)).toThrow()
    expect(scene.can_scene_undo()).toBe(false)

    scene.scene_redo()
    expect(scene.instance_ids()).toContain(instance)
    const members = scene.component_member_objects(comp)
    expect(scene.object_watertight(members[0])).toBe(true)
    expect(scene.can_scene_redo()).toBe(false)
  })
})
