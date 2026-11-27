/**
 * The Loft tool's start grips through the REAL compiled wasm module: three
 * rectangles of different proportions offer their corners as starts
 * (D-126), the grips sit on corners, and a drag lands on the nearest one.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as THREE from 'three'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { LoftTool } from './LoftTool'
import type { Ray } from '../viewport/math'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

/** A rectangle of half-sizes w × h at height z, drawn from (w, −h), with a
 *  point in the middle of its +y side: five vertices, four corners (so
 *  corner-matched starts and vertex starts differ). */
function rect(scene: Scene, w: number, h: number, z: number): void {
  const sketch = scene.begin_sketch_on_plane(0, 0, z, 0, 0, 1)
  const c: Array<[number, number]> = [
    [w, -h],
    [w, h],
    [0, h],
    [-w, h],
    [-w, -h],
  ]
  for (let i = 0; i < c.length; i++) {
    const a = c[i]
    const b = c[(i + 1) % c.length]
    scene.sketch_add_segment(sketch, a[0], a[1], z, b[0], b[1], z)
  }
}

/** A 32-side circle of radius r about the z axis at height z. */
function circle(scene: Scene, r: number, z: number): void {
  const sketch = scene.begin_sketch_on_plane(0, 0, z, 0, 0, 1)
  scene.sketch_begin_curve_with(sketch, 0, 0, z, r)
  const n = 32
  for (let i = 0; i < n; i++) {
    const a0 = (2 * Math.PI * i) / n
    const a1 = (2 * Math.PI * (i + 1)) / n
    scene.sketch_add_segment(sketch, r * Math.cos(a0), r * Math.sin(a0), z, r * Math.cos(a1), r * Math.sin(a1), z)
  }
  scene.sketch_end_curve(sketch)
}

const vertical = (x: number, y: number, fromAbove: boolean): Ray =>
  fromAbove ? { origin: [x, y, 10], direction: [0, 0, -1] } : { origin: [x, y, -10], direction: [0, 0, 1] }

/** The start grips' positions, in profile order. */
function grips(group: THREE.Group): THREE.Vector3[] {
  return group.children
    .filter((o) => o instanceof THREE.Mesh && o.renderOrder === 998)
    .map((o) => o.position.clone())
}

function click(tool: LoftTool, ray: Ray): void {
  tool.onPointerMove(null, ray)
  tool.onPointerDown(null, ray)
  tool.onPointerUp()
}

/** Press on grip `g` (from below its plane) and drag to `to`. */
function dragGrip(tool: LoftTool, g: THREE.Vector3, to: Ray): void {
  const onGrip: Ray = { origin: [g.x, g.y, g.z - 10], direction: [0, 0, 1] }
  tool.onPointerMove(null, onGrip)
  tool.onPointerDown(null, onGrip)
  tool.onPointerMove(null, to)
  tool.onPointerUp()
}

const dims: Array<[number, number, number]> = [
  [0.5, 0.3, 0],
  [0.6, 0.4, 0.5],
  [0.55, 0.35, 1],
]

describe('LoftTool start grips (wasm)', () => {
  it('sit on the corners of three rectangles, and a drag lands on the nearest corner, not a side point', () => {
    const scene = new Scene()
    for (const [w, h, z] of dims) rect(scene, w, h, z)
    const group = new THREE.Group()
    const tool = new LoftTool(scene, group, vi.fn(), vi.fn())
    // Bottom from below, the middle where only it lies, the top from above.
    for (const ray of [vertical(0, 0, false), vertical(0.58, 0.38, true), vertical(0, 0, true)]) click(tool, ray)
    const at = grips(group)
    expect(at).toHaveLength(3)
    at.forEach((p, k) => {
      const [w, h, z] = dims[k]
      expect(Math.abs(Math.abs(p.x) - w)).toBeLessThan(1e-9)
      expect(Math.abs(Math.abs(p.y) - h)).toBeLessThan(1e-9)
      expect(p.z).toBeCloseTo(z, 9)
    })
    // Toward (0.05, 0.28), beside the side point (0, 0.3): the nearest
    // start is the corner (0.5, 0.3), the side point being no start.
    dragGrip(tool, at[0], vertical(0.05, 0.28, false))
    const moved = grips(group)[0]
    expect(moved.x).toBeCloseTo(0.5, 9)
    expect(moved.y).toBeCloseTo(0.3, 9)
  })

  it('takes a rail between corners the automatic match does not pair, and lofts along it (playtest 3.3.2)', () => {
    // A 0.5 square on the ground; the same square turned 45° one up. The
    // automatic match pairs each ground corner with one of the two top
    // corners 45° from it; a rail from each of two neighboring ground
    // corners to the top corner (0, −r) covers the other pairing too.
    const r = 0.25 * Math.SQRT2
    for (const a of [[0.25, -0.25, 0], [-0.25, -0.25, 0]] as Array<[number, number, number]>) {
      const scene = new Scene()
      const ground = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
      const sq: Array<[number, number]> = [[0.25, -0.25], [0.25, 0.25], [-0.25, 0.25], [-0.25, -0.25]]
      for (let i = 0; i < 4; i++) {
        const p = sq[i]
        const q = sq[(i + 1) % 4]
        scene.sketch_add_segment(ground, p[0], p[1], 0, q[0], q[1], 0)
      }
      const top = scene.begin_sketch_on_plane(0, 0, 1, 0, 0, 1)
      const dia: Array<[number, number]> = [[r, 0], [0, r], [-r, 0], [0, -r]]
      for (let i = 0; i < 4; i++) {
        const p = dia[i]
        const q = dia[(i + 1) % 4]
        scene.sketch_add_segment(top, p[0], p[1], 1, q[0], q[1], 1)
      }
      // A rail in the upright plane through the ground corner and the top
      // corner (0, −r), bowed out of the straight line.
      const b: [number, number, number] = [0, -r, 1]
      const along = [b[0] - a[0], b[1] - a[1], 0]
      const len = Math.hypot(along[0], along[1])
      const n = [-along[1] / len, along[0] / len, 0]
      const railSketch = scene.begin_sketch_on_plane(a[0], a[1], a[2], n[0], n[1], n[2])
      // Across the chord within the rail's plane: (−1, len) in (along, up).
      const k = 0.15 / Math.hypot(1, len)
      const mid: [number, number, number] = [
        (a[0] + b[0]) / 2 - (k * along[0]) / len,
        (a[1] + b[1]) / 2 - (k * along[1]) / len,
        0.5 + k * len,
      ]
      scene.sketch_add_segment(railSketch, a[0], a[1], a[2], mid[0], mid[1], mid[2])
      scene.sketch_add_segment(railSketch, mid[0], mid[1], mid[2], b[0], b[1], b[2])
      const group = new THREE.Group()
      const onCommit = vi.fn()
      const onToast = vi.fn()
      const tool = new LoftTool(scene, group, onCommit, onToast)
      click(tool, vertical(0.1, 0.05, false))
      click(tool, vertical(0.1, 0.05, true))
      expect(grips(group)).toHaveLength(2)
      // Alt-click the rail's first segment, from the side.
      const hit = [(a[0] + mid[0]) / 2, (a[1] + mid[1]) / 2, (a[2] + mid[2]) / 2]
      tool.setAltModifier(true)
      click(tool, { origin: [hit[0] + n[0] * 5, hit[1] + n[1] * 5, hit[2]], direction: [-n[0], -n[1], 0] })
      tool.setAltModifier(false)
      expect(onToast).not.toHaveBeenCalled()
      // The rail decides the pairing: no start grips.
      expect(grips(group)).toHaveLength(0)
      tool.onKey(({ key: 'Enter', ctrlKey: false, metaKey: false } as KeyboardEvent))
      expect(onToast).not.toHaveBeenCalled()
      expect(onCommit).toHaveBeenCalledTimes(1)
    }
  })

  it("takes a rail drawn from a corner in the shape's own drawing as the rail alone", () => {
    // An upright rectangle standing on the ground and a ground rectangle
    // beside it; the rail runs on the ground from the ground rectangle's
    // corner to the upright one's foot, in the ground rectangle's own
    // sketch (it forks at the corner, which the whole drawing would).
    const scene = new Scene()
    const ground = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
    const sq: Array<[number, number]> = [[0.2, -0.2], [0.2, 0.2], [-0.2, 0.2], [-0.2, -0.2]]
    for (let i = 0; i < 4; i++) {
      const p = sq[i]
      const q = sq[(i + 1) % 4]
      scene.sketch_add_segment(ground, p[0], p[1], 0, q[0], q[1], 0)
    }
    // The upright one in the plane x = 1, from y −0.2 to 0.2, z 0 to 0.4.
    const wall = scene.begin_sketch_on_plane(1, 0, 0, 1, 0, 0)
    const up: Array<[number, number]> = [[-0.2, 0], [0.2, 0], [0.2, 0.4], [-0.2, 0.4]]
    for (let i = 0; i < 4; i++) {
      const p = up[i]
      const q = up[(i + 1) % 4]
      scene.sketch_add_segment(wall, 1, p[0], p[1], 1, q[0], q[1])
    }
    // The rail: (0.2, −0.2, 0) → (0.6, −0.3, 0) → (1, −0.2, 0).
    scene.sketch_add_segment(ground, 0.2, -0.2, 0, 0.6, -0.3, 0)
    scene.sketch_add_segment(ground, 0.6, -0.3, 0, 1, -0.2, 0)
    const group = new THREE.Group()
    const onCommit = vi.fn()
    const onToast = vi.fn()
    const tool = new LoftTool(scene, group, onCommit, onToast)
    click(tool, vertical(0, 0, true))
    click(tool, { origin: [5, 0, 0.2], direction: [-1, 0, 0] })
    tool.setAltModifier(true)
    click(tool, vertical(0.4, -0.25, true))
    tool.setAltModifier(false)
    expect(onToast).not.toHaveBeenCalled()
    tool.onKey(({ key: 'Enter', ctrlKey: false, metaKey: false } as KeyboardEvent))
    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('refuses a rail that misses a shape with the reason, and does not count it', () => {
    const scene = new Scene()
    rect(scene, 0.5, 0.3, 0)
    rect(scene, 0.5, 0.3, 1)
    // A loose line beside them, touching neither.
    const loose = scene.begin_sketch_on_plane(2, 0, 0, 1, 0, 0)
    scene.sketch_add_segment(loose, 2, 0, 0, 2, 0, 1)
    const group = new THREE.Group()
    const onToast = vi.fn()
    const tool = new LoftTool(scene, group, vi.fn(), onToast)
    click(tool, vertical(0.1, 0.05, false))
    click(tool, vertical(0.1, 0.05, true))
    tool.setAltModifier(true)
    const onLine: Ray = { origin: [2, -5, 0.5], direction: [0, 1, 0] }
    for (let i = 0; i < 3; i++) click(tool, onLine)
    expect(onToast).toHaveBeenCalledWith('Each rail must run through a matching corner or point on every shape you loft between, and two rails must line the shapes up the same way.', 'RailMissesProfile')
    expect(onToast).not.toHaveBeenCalledWith('A loft can follow at most two paths.')
  })

  it('keeps its grip where the loft starts when the starts change under a kept offset', () => {
    const scene = new Scene()
    rect(scene, 0.5, 0.3, 0)
    const plain = scene.begin_sketch_on_plane(0, 0, 1, 0, 0, 1)
    const c: Array<[number, number]> = [
      [0.6, -0.4],
      [0.6, 0.4],
      [-0.6, 0.4],
      [-0.6, -0.4],
    ]
    for (let i = 0; i < 4; i++) {
      const a = c[i]
      const b = c[(i + 1) % 4]
      scene.sketch_add_segment(plain, a[0], a[1], 1, b[0], b[1], 1)
    }
    circle(scene, 0.3, 2)
    const group = new THREE.Group()
    const tool = new LoftTool(scene, group, vi.fn(), vi.fn())
    click(tool, vertical(0, 0, false))
    click(tool, vertical(0.58, 0.38, true))
    click(tool, vertical(0, 0, true))
    // With the circle, the shapes match by perimeter: the first offers
    // all five vertices; its grip goes to the last, (−0.5, −0.3).
    dragGrip(tool, grips(group)[0], vertical(-0.5, -0.3, false))
    expect(grips(group)[0].x).toBeCloseTo(-0.5, 9)
    expect(grips(group)[0].y).toBeCloseTo(-0.3, 9)
    // Without it the two rectangles match corner to corner: four starts,
    // and the kept offset of four wraps to the first, as the kernel does.
    click(tool, vertical(0, 0, true))
    expect(grips(group)).toHaveLength(2)
    expect(grips(group)[0].x).toBeCloseTo(0.5, 9)
    expect(grips(group)[0].y).toBeCloseTo(-0.3, 9)
  })
})
