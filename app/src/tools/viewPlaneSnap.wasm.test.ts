/**
 * View-plane inference meets the real snap service and the REAL compiled
 * wasm kernel (docs/design/view-plane-inference.md §8, `snapConstraint`):
 * over empty space an idle draw tool infers the axis plane the view faces,
 * but hands it to the snap service as a `fallbackPlane` only. Were it sent
 * as a `constraintPlane`, the kernel would drop every candidate off that
 * plane, so a cube's corner standing behind the green plane could never be
 * snapped from a Front view, and the first click near it would land in
 * empty space instead of on the cube. Mirrors
 * `text/placeText.wasm.test.ts`'s real-wasm-in-vitest pattern.
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
import type { Snap, SnapConstraint } from './types'
import { LineTool } from './LineTool'
import { RectangleTool } from './RectangleTool'
import { CircleTool } from './CircleTool'
import { PolygonTool } from './PolygonTool'
import { ArcTool } from './ArcTool'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

type V3 = [number, number, number]

const W = 800
const H = 600
const FOV = 45
const BASIS: ApertureBasis = { kind: 'perspective', fovYDeg: FOV }

/** A 1 m cube with its lower front-left corner at (3, 2, 0). */
function sceneWithCube(): Scene {
  const scene = new Scene()
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const c: [number, number][] = [[3, 2], [4, 2], [4, 3], [3, 3]]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = c[i]
    const [bx, by] = c[(i + 1) % 4]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  expect(regions.length).toBe(1)
  scene.extrude_region(sketch, regions[0], 1)
  return scene
}

/** The Front standard view: level, looking along +Y, so the green plane
 *  y = 0 faces the camera and lies between the eye and the cube. */
function frontCamera(): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(FOV, W / H, 0.1, 1000)
  cam.position.set(2, -8, 0.5)
  cam.up.set(0, 0, 1)
  cam.lookAt(2, 0, 0.5)
  cam.updateMatrixWorld(true)
  cam.updateProjectionMatrix()
  return cam
}

/** Screen pixel of world point `p`. */
function toPixel(cam: THREE.Camera, p: V3): [number, number] {
  const v = new THREE.Vector3(...p).project(cam)
  return [((v.x + 1) / 2) * W, ((1 - v.y) / 2) * H]
}

/** The cursor ray through screen pixel (`px`, `py`). */
function rayAtPixel(cam: THREE.PerspectiveCamera, px: number, py: number): Ray {
  const ndc = new THREE.Vector3((px / W) * 2 - 1, 1 - (py / H) * 2, 0.5).unproject(cam)
  const d = ndc.sub(cam.position).normalize()
  return { origin: [cam.position.x, cam.position.y, cam.position.z], direction: [d.x, d.y, d.z] }
}

interface Under {
  updateCamera(camera: THREE.Camera): void
  snapConstraint(ray: Ray): SnapConstraint | null
  onPointerMove(snap: Snap | null, ray: Ray): void
  statusHint(): string
  _currentMode(ray?: Ray, snap?: Snap | null): 'face' | 'plane'
}

const noop = vi.fn()
const TOOLS: { name: string; make: (s: WasmScene) => Under }[] = [
  { name: 'Line', make: (s) => new LineTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Rectangle', make: (s) => new RectangleTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Circle', make: (s) => new CircleTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Polygon', make: (s) => new PolygonTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Arc', make: (s) => new ArcTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
]

/** One Viewport-shaped hover: ask the tool for its constraint, resolve the
 *  snap the way every Viewport call site does, then route the move. */
function hoverAt(tool: Under, svc: SnapService, ray: Ray): Snap | null {
  const c = tool.snapConstraint(ray)
  const { snap } = svc.resolve(
    ray, H, BASIS, c?.anchor, c?.lockAxis, c?.constraintPlane, c?.offPlanePoints,
    undefined, c?.facesOnly, false, c?.seeThroughPoints, c?.fallbackPlane,
  )
  tool.onPointerMove(snap, ray)
  return snap
}

describe.each(TOOLS)('$name — Front view over a cube at (3, 2, 0) (real kernel)', ({ make }) => {
  it('a hover just off the cube, within the snap radius of its corner, snaps the corner and starts on the cube', () => {
    const scene = sceneWithCube()
    const tool = make(scene as unknown as WasmScene)
    const svc = new SnapService(scene as unknown as WasmScene)
    const cam = frontCamera()
    tool.updateCamera(cam)
    // Warm the idle inference over empty space first: the green plane.
    hoverAt(tool, svc, rayAtPixel(cam, 40, 40))
    expect(tool.statusHint()).toContain('Facing the green plane')

    // The cube's upper front-left corner, nudged 4 px left and 4 px up:
    // outside the silhouette (the plain ray misses the cube), but within
    // the 8 px snap radius.
    const [cx, cy] = toPixel(cam, [3, 2, 1])
    const ray = rayAtPixel(cam, cx - 4, cy - 4)
    expect(scene.pick_face(...ray.origin, ...ray.direction)).toBeUndefined()
    const snap = hoverAt(tool, svc, ray)
    expect(snap).not.toBeNull()
    expect(snap!.kind).not.toBe('plane')
    expect(snap!.y).toBeCloseTo(2, 6)
    expect(tool._currentMode(ray, snap)).toBe('face')
    expect(tool.statusHint()).not.toContain('Facing')
    scene.free()
  })

  it('a hover over empty space lands on the green plane y = 0 (kind "plane")', () => {
    const scene = sceneWithCube()
    const tool = make(scene as unknown as WasmScene)
    const svc = new SnapService(scene as unknown as WasmScene)
    const cam = frontCamera()
    tool.updateCamera(cam)
    // Upper left of the screen, far from the cube and above the horizon,
    // so the ray never reaches the ground.
    const ray = rayAtPixel(cam, 60, 80)
    expect(ray.direction[2]).toBeGreaterThan(0)
    const snap = hoverAt(tool, svc, ray)
    expect(snap?.kind).toBe('plane')
    expect(snap!.y).toBeCloseTo(0, 9)
    expect(tool._currentMode(ray, snap)).toBe('plane')
    scene.free()
  })
})

/** A 1 m cube at (3, 2, 0) and a free vertical Line 2 m tall standing in
 *  front of it at (3.4, 1.5), drawn on the plane y = 1.5. From a high camera
 *  the Line's top end, in the air at z = 2, lies on screen over the cube's
 *  top face (z = 1). A drawn Line, not a solid post: a solid's corner is a
 *  point of the solid's own faces, so the tool's boundary pick takes one of
 *  those faces and none ever gives way. */
function sceneWithCubeAndLine(): Scene {
  const scene = sceneWithCube()
  const sketch = scene.begin_sketch_on_plane(3.4, 1.5, 0, 0, -1, 0)
  scene.sketch_begin_gesture(sketch)
  scene.sketch_add_segment(sketch, 3.4, 1.5, 0, 3.4, 1.5, 2)
  scene.sketch_end_gesture(sketch)
  return scene
}

function highCamera(): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(FOV, W / H, 0.1, 1000)
  cam.position.set(3.5, -1, 6)
  cam.up.set(0, 0, 1)
  cam.lookAt(3.5, 2.5, 0.5)
  cam.updateMatrixWorld(true)
  cam.updateProjectionMatrix()
  return cam
}

// Ruling R1 against the real engine; the guards below say so loudly if the
// fixture geometry is off.
describe.each(TOOLS)('$name — a face gives way to a named point in front of it (real kernel)', ({ make }) => {
  it("a hover on the cube's top face snapped to a Line's top end goes to plane mode", () => {
    const scene = sceneWithCubeAndLine()
    const tool = make(scene as unknown as WasmScene)
    const svc = new SnapService(scene as unknown as WasmScene)
    const cam = highCamera()
    tool.updateCamera(cam)
    const [cx, cy] = toPixel(cam, [3.4, 1.5, 2])
    // Nudge toward the cube, inside the snap radius of the Line's end.
    const ray = rayAtPixel(cam, cx + 2, cy + 2)
    const snap = hoverAt(tool, svc, ray)
    expect(snap).not.toBeNull()
    expect(snap!.z).toBeCloseTo(2, 6)
    expect(tool._currentMode(ray, snap)).toBe('plane')
    scene.free()
  })
})
