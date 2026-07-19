/**
 * ArcTool — sketch mode (sketches on any plane, Phase 2 wave B): hovering a
 * committed sketch that is NOT on the ground plane, at top level, adopts
 * that sketch's plane and targets it directly (`SketchTarget.existing`)
 * instead of the shared per-plane ground cache. See
 * the sketch-planes design §1/§4 and `drawPlane.ts`/`sketchGesture.ts`.
 * Mirrors LineTool.sketchMode.test.ts / RectangleTool.sketchMode.test.ts /
 * CircleTool.sketchMode.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ArcTool } from './ArcTool'
import { makeSketchPlaneCache, type SketchPlaneCache } from './sketchGesture'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'plane', ...overrides }
}

function makeKeyEvent(key: string, opts: { repeat?: boolean } = {}): KeyboardEvent {
  return { key, repeat: opts.repeat ?? false, preventDefault: () => {} } as unknown as KeyboardEvent
}

/** A tilted sketch: the XZ plane (y=0), normal +Y — a horizontal-but-rotated
 *  "wall" sketch whose in-plane basis spans X and Z (not X/Y like ground),
 *  so any drawn geometry has non-trivial Z coordinates. */
const TILTED_SKETCH = 55n
const TILTED_PLANE = new Float64Array([0, 0, 0, 0, 1, 0])
const GROUND_PLANE_ARR = new Float64Array([0, 0, 0, 0, 0, 1])
/** Plane-membership tolerance mirroring drawPlane.ts's GROUND_PLANE_EPS. */
const PLANE_EPS = 1e-9

type SegmentCall = [number, number, number, number, number, number]
type CurveWithCall = { sketch: bigint; cx: number; cy: number; cz: number; radius: number }

function makeWasmScene(opts: {
  /** `pick_sketch` result for every ray — a fixed handle (hit) or undefined (miss). */
  sketchPick?: bigint
  /** `sketch_plane` per handle; mutate to simulate a sketch vanishing mid-gesture. */
  planes?: Map<bigint, Float64Array | undefined>
} = {}) {
  const planes = opts.planes ?? new Map<bigint, Float64Array | undefined>([[TILTED_SKETCH, TILTED_PLANE]])
  const segments: SegmentCall[] = []
  const curveWithCalls: CurveWithCall[] = []
  let sketchCounter = 90n
  let nextRegionsCreated: bigint[] = []

  const scene = {
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      planes.set(sketchCounter, GROUND_PLANE_ARR)
      return sketchCounter
    }),
    // Not exercised by these fixtures (sketch mode never mints a
    // non-ground plane sketch — see sketchGesture.ts) but present so this
    // fake matches the real WasmScene signature (Phase 3).
    begin_sketch_on_plane: vi.fn(() => {
      throw new Error('begin_sketch_on_plane should not be called in sketch mode')
    }),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => opts.sketchPick),
    sketch_plane: vi.fn((h: bigint) => planes.get(h)),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_begin_curve: vi.fn(() => 91n),
    sketch_begin_curve_with: vi.fn((sketch: bigint, cx: number, cy: number, cz: number, radius: number) => {
      curveWithCalls.push({ sketch, cx, cy, cz, radius })
      return 91n
    }),
    sketch_end_curve: vi.fn(),
    sketch_add_segment: vi.fn(
      (sketch: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
        segments.push([ax, ay, az, bx, by, bz])
        // ArcTool reports the LAST segment's regions_created, so setting
        // this before the commit and never resetting it is enough (mirrors
        // RectangleTool.sketchMode.test.ts / CircleTool.sketchMode.test.ts).
        return {
          new_edges: () => new BigUint64Array([]),
          regions_created: () => new BigUint64Array(nextRegionsCreated),
          regions_removed: () => new BigUint64Array([]),
          free: vi.fn(),
        }
      },
    ),
  }
  return {
    scene: scene as unknown as WasmScene,
    segments,
    curveWithCalls,
    setNextRegionsCreated: (rc: bigint[]) => { nextRegionsCreated = rc },
    planes,
  }
}

function makeTool(scene: WasmScene, cache: SketchPlaneCache = makeSketchPlaneCache()) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new ArcTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, cache)
  return { tool, onCommit, onToast, onFaceImprint, onMeasurement, cache }
}

describe('ArcTool — sketch mode (drawing on a hovered non-ground sketch)', () => {
  it('hovering a rotated sketch anchors in sketch mode with its plane', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY) // endpoint A

    expect(tool.capturingInput()).toBe(true)
    expect(scene.pick_sketch).toHaveBeenCalled()
    // Sketch mode targets the EXISTING handle — no ground sketch minted.
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
  })

  it('a committed arc sends sketch_begin_curve_with an on-plane center and every segment endpoint on-plane, with exact A/B endpoints and non-trivial tilted-axis (Z) coordinates', () => {
    const { scene, segments, curveWithCalls } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY)   // A
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 2 }), RAY)   // B — chord (1,0,2)-(4,0,2)
    tool.onPointerMove(makeSnap({ x: 2.5, y: 0, z: 3.5 }), RAY) // pull the bulge, still on-plane (y=0)
    tool.onPointerDown(makeSnap({ x: 2.5, y: 0, z: 3.5 }), RAY) // commit

    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith({ sketchHandle: TILTED_SKETCH, regionsCreated: [] })

    expect(curveWithCalls).toHaveLength(1)
    expect(curveWithCalls[0].sketch).toBe(TILTED_SKETCH)
    expect(Math.abs(curveWithCalls[0].cy)).toBeLessThan(PLANE_EPS) // center on-plane

    expect(segments.length).toBeGreaterThan(1)
    // Exact endpoints — no float drift at the chain's ends (mirrors the
    // ground-mode "exact endpoints" assertion in ArcTool.test.ts).
    expect(segments[0][0]).toBe(1)
    expect(segments[0][1]).toBe(0)
    expect(segments[0][2]).toBe(2)
    expect(segments[segments.length - 1][3]).toBe(4)
    expect(segments[segments.length - 1][4]).toBe(0)
    expect(segments[segments.length - 1][5]).toBe(2)

    // Every endpoint satisfies |(p - origin)·normal| < 1e-9 — origin is
    // [0,0,0], normal is [0,1,0], so this is exactly "y ~ 0".
    for (const s of segments) {
      expect(Math.abs(s[1])).toBeLessThan(PLANE_EPS)
      expect(Math.abs(s[4])).toBeLessThan(PLANE_EPS)
    }
    // Non-trivial coordinates on the tilted axis: this plane's basis spans
    // X and Z (not X/Y like ground), so some Z values must differ from the
    // chord's own Z (2) — NOT the ground-mode z=0 fast path.
    const zValues = segments.flatMap((s) => [s[2], s[5]])
    expect(zValues.some((z) => Math.abs(z - 2) > 1e-6)).toBe(true)
  })

  it('a region-closing commit (pie completion) fires onCommit with the created region handles', () => {
    const { scene, setNextRegionsCreated } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY) // A
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 2 }), RAY) // B
    tool.onPointerMove(makeSnap({ x: 2.5, y: 0, z: 3.5 }), RAY)
    tool.onKey(makeKeyEvent('Alt')) // open → pie (closes the loop)
    setNextRegionsCreated([77n])
    tool.onPointerDown(makeSnap({ x: 2.5, y: 0, z: 3.5 }), RAY) // commit

    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith({ sketchHandle: TILTED_SKETCH, regionsCreated: [77n] })
  })

  it('a stale hovered sketch mid-gesture toasts and does NOT create a ground sketch', () => {
    const { scene, planes, segments } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY) // A — anchors sketch mode
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 2 }), RAY) // B — no commit yet
    planes.delete(TILTED_SKETCH) // the sketch vanished (e.g. its creating gesture was undone)

    tool.onPointerDown(makeSnap({ x: 2.5, y: 0, z: 3.5 }), RAY) // attempted commit (bulge)

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled() // never a silent retarget
    expect(segments.length).toBe(0)
  })

  it('ground drawing (no sketch under the cursor) still shares the per-plane cache across tool instances', () => {
    const { scene, segments } = makeWasmScene({ sketchPick: undefined })
    const cache = makeSketchPlaneCache()
    const { tool: arc1 } = makeTool(scene, cache)

    arc1.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    arc1.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    arc1.onPointerDown(makeSnap({ x: 1, y: 1 }), RAY)

    // A fresh tool instance (as after a tool switch) sharing the same cache
    // commits into the SAME ground sketch.
    const { tool: arc2 } = makeTool(scene, cache)
    arc2.onPointerDown(makeSnap({ x: 5, y: 0 }), RAY)
    arc2.onPointerDown(makeSnap({ x: 7, y: 0 }), RAY)
    arc2.onPointerDown(makeSnap({ x: 6, y: 1 }), RAY)

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    // The ground fast path: every committed z is exactly 0.
    expect(segments.every((s) => s[2] === 0 && s[5] === 0)).toBe(true)
  })
})
