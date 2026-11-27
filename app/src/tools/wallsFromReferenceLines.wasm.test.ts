/**
 * Four walls stood on a ground square with the mouse, against the REAL
 * compiled wasm kernel and snap service (wireframe-faces-and-solids design
 * §3.2 source 2, playtest step 4). Each wall is a Rectangle started at a
 * ground corner under an arrow-key plane lock, its far corner aimed by eye a
 * couple of pixels off the top corner, as a hand does.
 *
 * The first wall's height is wherever the cursor stood on the vertical
 * through the edge's far corner. Every later wall starts on a corner the
 * ground and the previous wall share, so both shapes lend reference lines:
 * the previous wall's top corner offers a level line on the new wall's plane,
 * and its crossing with the vertical through the far ground corner is the
 * new top corner — the first wall's height exactly. Were only the ground
 * Rectangle the start shape, each wall would take the cursor's own height and
 * the tops would miss each other by a fraction of a pixel: no top face, no
 * solid, and stray fills when the box is capped.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import * as THREE from 'three'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { SnapService } from '../viewport/snapService'
import type { ApertureBasis, Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { RectangleTool } from './RectangleTool'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

type V3 = [number, number, number]

const W = 1200
const H = 800
const FOV = 45
const BASIS: ApertureBasis = { kind: 'perspective', fovYDeg: FOV }

/** High over the front-right corner: every ground corner shows over the
 *  walls already standing (a corner hidden behind a wall is not offered). */
function camera(): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(FOV, W / H, 0.1, 1000)
  cam.position.set(2.0, -1.5, 4.5)
  cam.up.set(0, 0, 1)
  cam.lookAt(0.5, 0.5, 0.3)
  cam.updateMatrixWorld(true)
  cam.updateProjectionMatrix()
  return cam
}

function toPixel(cam: THREE.Camera, p: V3): [number, number] {
  const v = new THREE.Vector3(...p).project(cam)
  return [((v.x + 1) / 2) * W, ((1 - v.y) / 2) * H]
}

function rayAtPixel(cam: THREE.PerspectiveCamera, px: number, py: number): Ray {
  const ndc = new THREE.Vector3((px / W) * 2 - 1, 1 - (py / H) * 2, 0.5).unproject(cam)
  const d = ndc.sub(cam.position).normalize()
  return { origin: [cam.position.x, cam.position.y, cam.position.z], direction: [d.x, d.y, d.z] }
}

function key(k: string): KeyboardEvent {
  return { key: k, repeat: false, preventDefault: () => {} } as unknown as KeyboardEvent
}

/** The Viewport's pointer path: constraint, snap, then the tool. */
function pointer(tool: RectangleTool, svc: SnapService, cam: THREE.PerspectiveCamera, at: [number, number], down: boolean) {
  const ray = rayAtPixel(cam, at[0], at[1])
  const c = tool.snapConstraint(ray)
  const { snap } = svc.resolve(
    ray, H, BASIS, c?.anchor, c?.lockAxis, c?.lockDirection, c?.constraintPlane, c?.offPlanePoints,
    undefined, c?.facesOnly, false, c?.seeThroughPoints, c?.fallbackPlane,
  )
  tool.onPointerMove(snap, ray)
  if (down) tool.onPointerDown(snap, ray)
}

/** Every live free sketch's edges, exact. */
function freeSegments(scene: Scene): number[][] {
  const out: number[][] = []
  for (const s of scene.sketch_ids()) {
    if (scene.sketch_owner_object(s) !== undefined) continue
    for (const island of scene.sketch_island_ids(s)) {
      for (const e of scene.sketch_island_edges(s, island)) {
        const ends = scene.sketch_edge_endpoints(s, e)
        if (ends !== undefined) out.push(Array.from(ends))
      }
    }
  }
  return out
}

function topHeights(scene: Scene): number[] {
  const zs = new Set<number>()
  for (const s of freeSegments(scene)) for (const z of [s[2], s[5]]) if (z > 0.5) zs.add(z)
  return [...zs]
}

describe('four walls by mouse on a ground square (real kernel, reference lines)', () => {
  it('every wall takes the first wall\'s height to the bit, and the fourth closes the box into one solid', () => {
    const scene = new Scene()
    const cam = camera()
    const tool = new RectangleTool(scene as unknown as WasmScene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn())
    tool.updateCamera(cam)
    const fovRad = (FOV * Math.PI) / 180
    tool.updateDiskScale(cam, (dist) => (2 * dist * Math.tan(fovRad / 2)) / H)
    const svc = new SnapService(scene as unknown as WasmScene)
    const px = (p: V3, dx = 0, dy = 0): [number, number] => {
      const [x, y] = toPixel(cam, p)
      return [x + dx, y + dy]
    }

    // The ground square: two clicks, the second typed exactly.
    pointer(tool, svc, cam, px([0, 0, 0]), false)
    pointer(tool, svc, cam, px([0, 0, 0]), true)
    pointer(tool, svc, cam, px([1, 1, 0]), false)
    for (const ch of '1,1') tool.onKey(key(ch))
    tool.onKey(key('Enter'))
    expect(freeSegments(scene)).toHaveLength(4)

    // [start corner, plane lock, far top corner, the hand's error in px]
    const walls: [V3, string, V3, [number, number]][] = [
      [[0, 0, 0], 'ArrowLeft', [1, 0, 1], [1.5, -2]],
      [[1, 0, 0], 'ArrowRight', [1, 1, 1], [-2, 1.5]],
      [[1, 1, 0], 'ArrowLeft', [0, 1, 1], [2, 2]],
      [[0, 1, 0], 'ArrowRight', [0, 0, 1], [-1.5, -1]],
    ]
    walls.forEach(([start, lock, far, [dx, dy]], i) => {
      pointer(tool, svc, cam, px(start, 3, 3), false)
      tool.onKey(key(lock))
      pointer(tool, svc, cam, px(start), false)
      pointer(tool, svc, cam, px(start), true)
      pointer(tool, svc, cam, px(far, dx + 4, dy + 4), false)
      pointer(tool, svc, cam, px(far, dx, dy), false)
      pointer(tool, svc, cam, px(far, dx, dy), true)
      tool.onKey(key(lock)) // release the lock
      if (i < 3) {
        const zs = topHeights(scene)
        expect(zs, `after wall ${i + 1}: one top height, ${JSON.stringify(zs)}`).toHaveLength(1)
        expect(Math.abs(zs[0] - 1)).toBeLessThan(0.03)
      }
    })

    // The fourth wall closed the top loop: the close pass capped it and the
    // six faces became one solid, leaving no drawn faces behind.
    const objects = Array.from(scene.object_ids())
    expect(objects, `free segments left: ${JSON.stringify(freeSegments(scene))}`).toHaveLength(1)
    expect(scene.object_solid(objects[0])).toBe(true)
    expect(freeSegments(scene)).toEqual([])
    scene.free()
  })
})
