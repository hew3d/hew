/**
 * R18 against the real engine and the REAL compiled wasm kernel: named
 * points on the view-inferred plane and under a Shift pin (Kurt's playtest
 * find). A 1 m Rectangle lies on the ground; the camera stands low to one
 * side so the picker infers the green (or red) plane. At a corner of the
 * ground Rectangle that is off that plane, every draw tool must offer the
 * corner ("Endpoint", no "projected") and start its gesture on the inferred
 * plane's orientation THROUGH the corner; pinned or not; with no arrow key.
 * An ordinary (blue) view and an up-arrow lock keep the ground.
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
import type { DrawPlaneState } from './drawPlane'
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

/** A free 1 m Rectangle on the ground: (0,0)-(1,1). */
function sceneWithGroundRect(): Scene {
  const scene = new Scene()
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const c: [number, number][] = [[0, 0], [1, 0], [1, 1], [0, 1]]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = c[i]
    const [bx, by] = c[(i + 1) % 4]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  expect(Array.from(scene.sketch_regions(sketch)).length).toBe(1)
  return scene
}

function camera(eye: V3, target: V3): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(FOV, W / H, 0.1, 1000)
  cam.position.set(...eye)
  cam.up.set(0, 0, 1)
  cam.lookAt(...target)
  cam.updateMatrixWorld(true)
  cam.updateProjectionMatrix()
  return cam
}

/** Low and to the left of the Rectangle, looking along +Y: green inferred. */
const lowLeft = (): THREE.PerspectiveCamera => camera([-1.5, -6, 1], [0.5, 0.5, 0])
/** Low and to the right, looking along -X: red inferred. */
const lowRight = (): THREE.PerspectiveCamera => camera([7, -1.5, 1], [0.5, 0.5, 0])
/** An ordinary three-quarter view from above: blue. */
const home = (): THREE.PerspectiveCamera => camera([3.5, -3, 2.5], [0.5, 0.5, 0])

function toPixel(cam: THREE.Camera, p: V3): [number, number] {
  const v = new THREE.Vector3(...p).project(cam)
  return [((v.x + 1) / 2) * W, ((1 - v.y) / 2) * H]
}

function rayAtPixel(cam: THREE.PerspectiveCamera, px: number, py: number): Ray {
  const ndc = new THREE.Vector3((px / W) * 2 - 1, 1 - (py / H) * 2, 0.5).unproject(cam)
  const d = ndc.sub(cam.position).normalize()
  return { origin: [cam.position.x, cam.position.y, cam.position.z], direction: [d.x, d.y, d.z] }
}

interface Under {
  updateCamera(camera: THREE.Camera): void
  snapConstraint(ray: Ray): SnapConstraint | null
  onPointerMove(snap: Snap | null, ray: Ray): void
  onPointerDown(snap: Snap | null, ray: Ray): void
  onKey(ev: KeyboardEvent): void
  setShiftHeld(held: boolean): void
  snapProjected(): boolean
  drawPlaneState(): DrawPlaneState
}

const noop = vi.fn()
const TOOLS: { name: string; make: (s: WasmScene) => Under }[] = [
  { name: 'Line', make: (s) => new LineTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Rectangle', make: (s) => new RectangleTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Circle', make: (s) => new CircleTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Polygon', make: (s) => new PolygonTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Arc', make: (s) => new ArcTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
]

function hoverAt(tool: Under, svc: SnapService, ray: Ray): Snap | null {
  const c = tool.snapConstraint(ray)
  const { snap } = svc.resolve(
    ray, H, BASIS, c?.anchor, c?.lockAxis, c?.constraintPlane, c?.offPlanePoints,
    undefined, c?.facesOnly, false, c?.seeThroughPoints, c?.fallbackPlane,
  )
  tool.onPointerMove(snap, ray)
  return snap
}

/** The gesture's frozen plane and first point, whatever the tool calls them
 *  (`anchor`, a circle's or polygon's `center`, an arc's chord end `a`). */
function stageOf(tool: Under): { started: boolean; plane: { normal: V3; ground: boolean }; anchor: V3 } {
  const st = (tool as unknown as { planeStage: Record<string, unknown> }).planeStage
  const anchor = (st.anchor ?? st.center ?? st.a) as V3
  return { started: st.kind !== 'idle', plane: st.plane as { normal: V3; ground: boolean }, anchor }
}

const key = (k: string): KeyboardEvent =>
  ({ key: k, repeat: false, preventDefault: () => { /* no-op */ } }) as unknown as KeyboardEvent

function expectVec(a: ArrayLike<number>, b: V3): void {
  for (let i = 0; i < 3; i++) expect(a[i]).toBeCloseTo(b[i], 6)
}

/** Which axis a unit normal runs along (sign ignored), or -1. */
function axisOf(n: V3): number {
  const a = n.map(Math.abs)
  return a.findIndex((x) => x > 1 - 1e-9)
}

interface Case {
  label: string
  cam: () => THREE.PerspectiveCamera
  /** The inferred orientation's axis: 0 red, 1 green. */
  axis: 0 | 1
  corners: V3[]
}

// Kurt's sequence: from the low left the corners off y = 0 are the far ones
// (y = 1); from the low right the corners off x = 0 are the near ones (x = 1).
// The corners ON the inferred plane (through the origin) are in the cases
// too: they start the same plane through the corner.
const CASES: Case[] = [
  { label: 'low left (green)', cam: lowLeft, axis: 1, corners: [[0, 1, 0], [1, 1, 0], [0, 0, 0], [1, 0, 0]] },
  { label: 'low right (red)', cam: lowRight, axis: 0, corners: [[1, 0, 0], [1, 1, 0], [0, 0, 0], [0, 1, 0]] },
]

describe.each(TOOLS)('$name — named points of a ground Rectangle on the inferred plane (real kernel)', ({ make }) => {
  describe.each(CASES)('$label', ({ cam: makeCam, axis, corners }) => {
    it.each(corners.map((c) => [c]))('unpinned: the corner is offered as "Endpoint", not projected, and starts on the plane through it (%j)', (corner) => {
      const scene = sceneWithGroundRect()
      const tool = make(scene as unknown as WasmScene)
      const svc = new SnapService(scene as unknown as WasmScene)
      const cam = makeCam()
      tool.updateCamera(cam)
      // Warm the idle hover over empty space: the inferred plane.
      hoverAt(tool, svc, rayAtPixel(cam, 40, 40))
      const at = corner as V3
      const [px, py] = toPixel(cam, at)
      const ray = rayAtPixel(cam, px, py)
      const snap = hoverAt(tool, svc, ray)
      expect(snap?.kind).toBe('endpoint')
      expectVec([snap!.x, snap!.y, snap!.z], at)
      expect(tool.snapProjected()).toBe(false)
      // The plane state passes through the corner and has the inferred
      // orientation.
      const state = tool.drawPlaneState()
      expect(state.hover).not.toBeNull()
      expect(axisOf(state.plane.normal)).toBe(axis)
      const o = state.plane.origin
      const n = state.plane.normal
      expect((at[0] - o[0]) * n[0] + (at[1] - o[1]) * n[1] + (at[2] - o[2]) * n[2]).toBeCloseTo(0, 6)
      tool.onPointerDown(snap, ray)
      const stage = stageOf(tool)
      expect(stage.started).toBe(true)
      expect(stage.plane.ground).toBe(false)
      expect(axisOf(stage.plane.normal)).toBe(axis)
      expectVec(stage.anchor, at)
      scene.free()
    })

    it.each(corners.map((c) => [c]))('Shift-pinned on the inferred plane: the corner is still offered and starts through it (%j)', (corner) => {
      const scene = sceneWithGroundRect()
      const tool = make(scene as unknown as WasmScene)
      const svc = new SnapService(scene as unknown as WasmScene)
      const cam = makeCam()
      tool.updateCamera(cam)
      hoverAt(tool, svc, rayAtPixel(cam, 40, 40))
      tool.setShiftHeld(true) // pins the plane the patch shows
      tool.setShiftHeld(false)
      const at = corner as V3
      const [px, py] = toPixel(cam, at)
      const ray = rayAtPixel(cam, px, py)
      const snap = hoverAt(tool, svc, ray)
      expect(snap?.kind).toBe('endpoint')
      expectVec([snap!.x, snap!.y, snap!.z], at)
      expect(tool.snapProjected()).toBe(false)
      tool.onPointerDown(snap, ray)
      const stage = stageOf(tool)
      expect(stage.started).toBe(true)
      expect(axisOf(stage.plane.normal)).toBe(axis)
      expectVec(stage.anchor, at)
      scene.free()
    })
  })

  it('an up-arrow still forces the ground at a corner, from a low view', () => {
    const scene = sceneWithGroundRect()
    const tool = make(scene as unknown as WasmScene)
    const svc = new SnapService(scene as unknown as WasmScene)
    const cam = lowLeft()
    tool.updateCamera(cam)
    tool.onKey(key('ArrowUp'))
    const [px, py] = toPixel(cam, [0, 1, 0])
    const ray = rayAtPixel(cam, px, py)
    const snap = hoverAt(tool, svc, ray)
    tool.onPointerDown(snap, ray)
    const stage = stageOf(tool)
    expect(stage.started).toBe(true)
    expect(stage.plane.ground).toBe(true)
    expectVec(stage.anchor, [0, 1, 0])
    scene.free()
  })

  it('an ordinary (blue) view keeps the ground at a corner of the ground Rectangle', () => {
    const scene = sceneWithGroundRect()
    const tool = make(scene as unknown as WasmScene)
    const svc = new SnapService(scene as unknown as WasmScene)
    const cam = home()
    tool.updateCamera(cam)
    hoverAt(tool, svc, rayAtPixel(cam, 40, 40))
    const [px, py] = toPixel(cam, [1, 1, 0])
    const ray = rayAtPixel(cam, px, py)
    const snap = hoverAt(tool, svc, ray)
    expect(snap?.kind).toBe('endpoint')
    expect(tool.snapProjected()).toBe(false)
    tool.onPointerDown(snap, ray)
    const stage = stageOf(tool)
    expect(stage.plane.ground).toBe(true)
    expectVec(stage.anchor, [1, 1, 0])
    scene.free()
  })

  it('over the Rectangle\'s fill (no named point) from a low view keeps the ground', () => {
    const scene = sceneWithGroundRect()
    const tool = make(scene as unknown as WasmScene)
    const svc = new SnapService(scene as unknown as WasmScene)
    const cam = lowLeft()
    tool.updateCamera(cam)
    const [px, py] = toPixel(cam, [0.5, 0.5, 0])
    const ray = rayAtPixel(cam, px, py)
    const snap = hoverAt(tool, svc, ray)
    tool.onPointerDown(snap, ray)
    const stage = stageOf(tool)
    expect(stage.started).toBe(true)
    expect(stage.plane.ground).toBe(true)
    scene.free()
  })
})
