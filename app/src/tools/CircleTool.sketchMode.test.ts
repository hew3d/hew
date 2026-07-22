/**
 * CircleTool — sketch mode (sketches on any plane, Phase 2 wave B): hovering
 * a committed sketch that is NOT on the ground plane, at top level, adopts
 * that sketch's plane and targets it directly (`SketchTarget.existing`)
 * instead of the shared per-plane ground cache. See
 * the sketch-planes design §1/§4 and `drawPlane.ts`/`sketchGesture.ts`.
 * Mirrors LineTool.sketchMode.test.ts / RectangleTool.sketchMode.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { CircleTool } from './CircleTool'
import { makeSketchPlaneCache, type SketchPlaneCache } from './sketchGesture'
import { segmentsPerTurn } from './arcMath'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

/**
 * A ray that genuinely PIERCES the tilted sketch's plane on the sketch
 * itself — what the pick cone models and what `rayLandsOnSketch` (the
 * sketch-mode adoption gate in `drawPlane.ts`) measures. It pierces y = 0 at
 * (1, 0, 0), a point on `SKETCH_LINES`'s bottom edge. A ray lying IN the
 * plane (the old `[0,0,5] → -Z` fixture) can never pick that plane's sketch
 * in the real app: it has no pierce point at all.
 */
const RAY: Ray = { origin: [1, 5, 0], direction: [0, -1, 0] }

/** The tilted sketch's own geometry: a 4 m square on the y = 0 plane, the
 *  shape `RAY` lands on. `Scene.sketch_lines` reports xyz endpoint pairs. */
const SKETCH_LINES = new Float32Array([
  0, 0, 0, 4, 0, 0,
  4, 0, 0, 4, 0, 4,
  4, 0, 4, 0, 0, 4,
  0, 0, 4, 0, 0, 0,
])

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'plane', ...overrides }
}

/** A tilted sketch: the XZ plane (y=0), normal +Y — a horizontal-but-rotated
 *  "wall" sketch whose in-plane basis spans X and Z (not X/Y like ground),
 *  so any drawn geometry has non-trivial Z coordinates. */
const TILTED_SKETCH = 55n
const TILTED_PLANE = new Float64Array([0, 0, 0, 0, 1, 0])
const GROUND_PLANE_ARR = new Float64Array([0, 0, 0, 0, 0, 1])
/** Plane-membership tolerance mirroring drawPlane.ts's GROUND_PLANE_EPS. */
const PLANE_EPS = 1e-9

type SegmentCall = { sketch: bigint; a: [number, number, number]; b: [number, number, number] }
type CurveWithCall = { sketch: bigint; cx: number; cy: number; cz: number; radius: number }

function makeWasmScene(opts: {
  /** `pick_sketch` result for every ray — a fixed handle (hit) or undefined (miss). */
  sketchPick?: bigint
  /** `sketch_plane` per handle; mutate to simulate a sketch vanishing mid-gesture. */
  planes?: Map<bigint, Float64Array | undefined>
} = {}) {
  const planes = opts.planes ?? new Map<bigint, Float64Array | undefined>([[TILTED_SKETCH, TILTED_PLANE]])
  const segmentCalls: SegmentCall[] = []
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
    // Mirrors the real `Scene.sketch_lines`: the sketch's segments, and a
    // throw for a sketch that is no longer there.
    sketch_lines: vi.fn((h: bigint) => {
      if (planes.get(h) === undefined) throw new Error('UnknownSketch')
      return SKETCH_LINES
    }),
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
        segmentCalls.push({ sketch, a: [ax, ay, az], b: [bx, by, bz] })
        // CircleTool reports the LAST segment's regions_created, so setting
        // this before the commit and never resetting it is enough (mirrors
        // RectangleTool.sketchMode.test.ts).
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
    segmentCalls,
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
  const tool = new CircleTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, cache)
  return { tool, onCommit, onToast, onFaceImprint, onMeasurement, cache }
}

describe('CircleTool — sketch mode (drawing on a hovered non-ground sketch)', () => {
  it('hovering a rotated sketch anchors in sketch mode with its plane', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY) // center

    expect(tool.capturingInput()).toBe(true)
    expect(scene.pick_sketch).toHaveBeenCalled()
    // Sketch mode targets the EXISTING handle — no ground sketch minted.
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
  })

  it('a GRAZING pick the ray does not land on draws on the ground, not the tilted sketch', () => {
    // The regression behind the Follow Me L-path spec: `pick_sketch`'s cone
    // measures perpendicular distance to the ray, so a standing sketch seen
    // near edge-on "hits" for a click aimed at the ground half a metre away.
    // `rayLandsOnSketch` (drawPlane.ts) rejects it — this ray pierces y = 0
    // at (1, 0, -20), nowhere near SKETCH_LINES. Pinned per tool so a wrapper
    // that stopped delegating to the shared resolver would be caught here and
    // not only in drawPlane.test.ts.
    const grazing: Ray = { origin: [1, 5, 0], direction: [0, -1, -4] }
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), grazing) // center
    tool.onPointerDown(makeSnap({ x: 4, y: 1, z: 0 }), grazing) // rim — commits

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(scene.sketch_begin_gesture).not.toHaveBeenCalledWith(TILTED_SKETCH)
  })

  it('a committed circle sends sketch_begin_curve_with an on-plane center and every segment endpoint on-plane, with non-trivial tilted-axis (Z) coordinates', () => {
    const { scene, segmentCalls, curveWithCalls } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY) // center
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 2 }), RAY) // rim, radius 3

    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(curveWithCalls).toHaveLength(1)
    expect(curveWithCalls[0].sketch).toBe(TILTED_SKETCH)
    // The curve's own center lies on the tilted plane (y ~ 0).
    expect(Math.abs(curveWithCalls[0].cy)).toBeLessThan(PLANE_EPS)
    expect(curveWithCalls[0].radius).toBeCloseTo(3, 9)

    expect(segmentCalls.length).toBe(segmentsPerTurn(3))
    expect(segmentCalls.every((s) => s.sketch === TILTED_SKETCH)).toBe(true)
    // Every endpoint satisfies |(p - origin)·normal| < 1e-9 — origin is
    // [0,0,0], normal is [0,1,0], so this is exactly "y ~ 0".
    for (const s of segmentCalls) {
      expect(Math.abs(s.a[1])).toBeLessThan(PLANE_EPS)
      expect(Math.abs(s.b[1])).toBeLessThan(PLANE_EPS)
    }
    // Non-trivial coordinates on the tilted axis: this plane's basis spans
    // X and Z (not X/Y like ground), so some Z values must be non-zero —
    // NOT the ground-mode z=0 fast path.
    const zValues = segmentCalls.flatMap((s) => [s.a[2], s.b[2]])
    expect(zValues.some((z) => Math.abs(z) > 1e-6)).toBe(true)

    expect(onCommit).toHaveBeenCalledWith({ sketchHandle: TILTED_SKETCH, regionsCreated: [] })
  })

  it('a region-closing final segment fires onCommit with the created region handles', () => {
    const { scene, setNextRegionsCreated } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onCommit } = makeTool(scene)

    setNextRegionsCreated([77n]) // the circle's closing facet reports a new region
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY) // center
    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 2 }), RAY) // rim — commits

    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith({ sketchHandle: TILTED_SKETCH, regionsCreated: [77n] })
  })

  it('a stale hovered sketch mid-gesture toasts and does NOT create a ground sketch', () => {
    const { scene, planes } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY) // anchors sketch mode (center)
    planes.delete(TILTED_SKETCH) // the sketch vanished (e.g. its creating gesture was undone)

    tool.onPointerDown(makeSnap({ x: 4, y: 0, z: 2 }), RAY) // attempted commit (rim)

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled() // never a silent retarget
    expect(scene.sketch_add_segment).not.toHaveBeenCalled()
  })

  it('ground drawing (no sketch under the cursor) still shares the per-plane cache across tool instances', () => {
    const { scene, segmentCalls } = makeWasmScene({ sketchPick: undefined })
    const cache = makeSketchPlaneCache()
    const { tool: circle1 } = makeTool(scene, cache)

    circle1.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    circle1.onPointerDown(makeSnap({ x: 2, y: 0, z: 0 }), RAY)

    // A fresh tool instance (as after a tool switch) sharing the same cache
    // commits into the SAME ground sketch.
    const { tool: circle2 } = makeTool(scene, cache)
    circle2.onPointerDown(makeSnap({ x: 5, y: 5, z: 0 }), RAY)
    circle2.onPointerDown(makeSnap({ x: 7, y: 5, z: 0 }), RAY)

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(new Set(segmentCalls.map((s) => s.sketch)).size).toBe(1)
    // The ground fast path: committed z is exactly 0.
    expect(segmentCalls.every((s) => s.a[2] === 0 && s.b[2] === 0)).toBe(true)
  })
})
