/**
 * The Deform tool driven through the REAL compiled wasm module: a Bend
 * dragged past its limit, on a plain bar (clamped) and after a Twist (the
 * limit search too dear for the drag; the commit names it, F-58), and an
 * object too heavy to preview live (F-41).
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as THREE from 'three'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { DeformTool, DEFORM_PREVIEW_SKIPPED_HINT } from './DeformTool'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import type { DeformMode } from './deformMode'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

/** A closed ground polygon extruded `h`. */
function extrude(scene: Scene, pts: Array<[number, number]>, h: number, curve?: { r: number }): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  if (curve !== undefined) scene.sketch_begin_curve_with(sketch, 0, 0, 0, curve.r)
  for (let i = 0; i < pts.length; i++) {
    const a = pts[i]
    const b = pts[(i + 1) % pts.length]
    scene.sketch_add_segment(sketch, a[0], a[1], 0, b[0], b[1], 0)
  }
  if (curve !== undefined) scene.sketch_end_curve(sketch)
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0], h)
}

function bounds(scene: Scene, id: bigint): number[] {
  return Array.from(scene.object_bounds(id, undefined))
}

/** The renderer's group for the object: a box over its bounds is all the
 *  tool reads from it. */
function objectsGroupFor(scene: Scene, id: bigint): THREE.Group {
  const b = bounds(scene, id)
  const root = new THREE.Group()
  const g = new THREE.Group()
  g.name = `Object_${id}`
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(b[3] - b[0], b[4] - b[1], b[5] - b[2]), new THREE.MeshBasicMaterial())
  mesh.position.set((b[0] + b[3]) / 2, (b[1] + b[4]) / 2, (b[2] + b[5]) / 2)
  g.add(mesh)
  root.add(g)
  root.updateMatrixWorld(true)
  return root
}

function key(k: string): KeyboardEvent {
  return { key: k, ctrlKey: false, metaKey: false } as KeyboardEvent
}

/** A ray along −x, onto the +x end of the box, through (y, z). */
const fromPlusX = (y: number, z: number): Ray => ({ origin: [5, y, z], direction: [-1, 0, 0] })

function makeTool(scene: Scene, id: bigint, mode: DeformMode) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new DeformTool(
    scene as unknown as WasmScene,
    preview,
    objectsGroupFor(scene, id),
    [{ kind: 'object', id }],
    onCommit,
    onToast,
    onMeasurement,
    mode,
  )
  return { tool, preview, onCommit, onToast, onMeasurement }
}

/** A 1 m × 0.5 m × 40 mm bar along x from the origin. */
function wideBar(scene: Scene): bigint {
  return extrude(scene, [[0, -0.25], [1, -0.25], [1, 0.25], [0, 0.25]], 0.04)
}

/** Types `text` into the tool, then Enter. */
function typeIn(tool: DeformTool, text: string): void {
  for (const c of text) tool.onKey(key(c))
  tool.onKey(key('Enter'))
}

describe('DeformTool (real kernel) — a Bend dragged past its limit', () => {
  it('clamps a plain bar at the limit it names, and that limit typed commits', () => {
    const scene = new Scene()
    // A bend toward the bar's width folds past about 229° (the bend center
    // reaches the far edge): a search the drag can afford on a plain bar.
    const id = wideBar(scene)
    const t = makeTool(scene, id, 'bend')
    const b = bounds(scene, id)
    const cy = (b[1] + b[4]) / 2
    const cz = (b[2] + b[5]) / 2
    t.tool.onPointerMove(null, fromPlusX(cy, cz))
    t.tool.onPointerDown(null, fromPlusX(cy, cz))
    // Toward −y by 2.5 m: an asked bend of 286° (2·2.5/1 rad).
    t.tool.onPointerMove(null, fromPlusX(cy - 0.05, cz))
    t.tool.onPointerMove(null, fromPlusX(cy - 2.5, cz))
    const hint = t.tool.statusHint()
    const m = /Angle limited to ([0-9.]+)°\./.exec(hint)
    expect(m, hint).not.toBeNull()
    const named = Number(m![1])
    expect(named).toBeGreaterThan(200)
    expect(named).toBeLessThan(286)
    expect(t.onMeasurement).toHaveBeenLastCalledWith(`${named}°`)
    const before = scene.history_generation()
    typeIn(t.tool, String(named))
    expect(t.onToast).not.toHaveBeenCalled()
    expect(t.onCommit).toHaveBeenCalledWith(id)
    expect(scene.history_generation()).not.toBe(before)
    expect(bounds(scene, id)[1]).toBeLessThan(b[1] - 0.1)
    scene.free()
  }, 60_000)

  it('after a Twist skips the preview past the limit; the click names the limit, which typed bends that way and commits (F-58)', () => {
    const scene = new Scene()
    // The bar twisted 30° about its length: its limit search builds the
    // bend some forty times, more than a drag can wait for, so the drag
    // past the limit skips the preview instead of holding the pointer.
    const id = wideBar(scene)
    scene.deform(id, JSON.stringify({
      mode: 'twist',
      frame: { origin: [0, 0, 0.02], axis: [1, 0, 0], u: [0, 1, 0], length: 1 },
      angle: (30 * Math.PI) / 180,
    })).free()
    const t = makeTool(scene, id, 'bend')
    const b = bounds(scene, id)
    const cy = (b[1] + b[4]) / 2
    const cz = (b[2] + b[5]) / 2
    t.tool.onPointerMove(null, fromPlusX(cy, cz))
    t.tool.onPointerDown(null, fromPlusX(cy, cz))
    t.tool.onPointerMove(null, fromPlusX(cy - 0.05, cz))
    t.tool.onPointerMove(null, fromPlusX(cy - 2.5, cz))
    expect(t.tool.statusHint()).toContain(DEFORM_PREVIEW_SKIPPED_HINT)
    expect(t.preview.children.length).toBe(0)
    // The click commits nothing and names the limit the commit found.
    const before = scene.history_generation()
    t.tool.onPointerDown(null, fromPlusX(cy - 2.5, cz))
    expect(scene.history_generation()).toBe(before)
    expect(t.onToast).toHaveBeenCalledTimes(1)
    const toast = String(t.onToast.mock.calls[0][0])
    const m = /Angle limited to ([0-9.]+)°\./.exec(toast)
    expect(m, toast).not.toBeNull()
    const named = Number(m![1])
    expect(named).toBeGreaterThan(200)
    expect(named).toBeLessThan(286)
    // The named limit, typed: within 1% of the true one, so it goes in at
    // exactly the limit, on the side the drag was on.
    typeIn(t.tool, String(named))
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onCommit).toHaveBeenCalledWith(id)
    expect(scene.history_generation()).not.toBe(before)
    // Bent toward −y: the moving end swung round to the −y side.
    expect(bounds(scene, id)[1]).toBeLessThan(b[1] - 0.1)
    scene.free()
  }, 60_000)
})

describe('DeformTool (real kernel) — an object too heavy to preview', () => {
  it('previews while it can, then skips for size and stops asking; the click still twists it', () => {
    const scene = new Scene()
    // A 300-facet cylinder 1 m long: a twist of 60° previews; by 240° the
    // bands through 300 long facets pass the preview's work cap.
    const n = 300
    const pts: Array<[number, number]> = []
    for (let i = 0; i < n; i++) {
      const a = (2 * Math.PI * i) / n
      pts.push([0.1 * Math.cos(a), 0.1 * Math.sin(a)])
    }
    const id = extrude(scene, pts, 1)
    const previews = vi.spyOn(scene, 'deform_preview')
    const limits = vi.spyOn(scene, 'deform_preview_limit')
    const t = makeTool(scene, id, 'twist')
    // The top end of the box is the moving end: a ray straight down.
    const down = (deg: number): Ray => {
      const a = (deg * Math.PI) / 180
      return { origin: [0.05 * Math.cos(a), 0.05 * Math.sin(a), 5], direction: [0, 0, -1] }
    }
    t.tool.onPointerMove(null, down(0))
    t.tool.onPointerDown(null, down(0))
    t.tool.onPointerMove(null, down(60))
    expect(t.tool.statusHint()).not.toContain(DEFORM_PREVIEW_SKIPPED_HINT)
    expect(t.preview.children.length).toBeGreaterThan(0)
    const before = previews.mock.calls.length
    t.tool.onPointerMove(null, down(240))
    expect(t.tool.statusHint()).toMatch(/^Preview skipped: .* Move to twist, click to finish, or type an angle \(25°\)\. Esc backs up\.$/)
    expect(previews.mock.calls.length).toBe(before + 1)
    // From here the drag asks the kernel nothing; the amount still follows.
    t.tool.onPointerMove(null, down(300))
    t.tool.onPointerMove(null, down(250))
    expect(previews.mock.calls.length).toBe(before + 1)
    expect(limits).not.toHaveBeenCalled()
    expect(t.onToast).not.toHaveBeenCalled()
    expect(t.onMeasurement).toHaveBeenLastCalledWith('250°')
    // Back to a light amount (the skip holds for the session), then click:
    // the commit builds it behind the busy card.
    t.tool.onPointerMove(null, down(150))
    t.tool.onPointerMove(null, down(60))
    expect(previews.mock.calls.length).toBe(before + 1)
    expect(t.onMeasurement).toHaveBeenLastCalledWith('60°')
    const busy = vi.fn((_m: string, work: () => void) => work())
    t.tool.setBusyRunner(busy)
    const generation = scene.history_generation()
    t.tool.onPointerDown(null, down(60))
    expect(busy).toHaveBeenCalledWith('Twisting…', expect.any(Function))
    expect(t.onToast).not.toHaveBeenCalled()
    expect(t.onCommit).toHaveBeenCalledWith(id)
    expect(scene.history_generation()).not.toBe(generation)
    scene.free()
  }, 60_000)
})
