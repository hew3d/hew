/**
 * LineTool — sketch mode (sketches on any plane, Phase 2 wave A): hovering a
 * committed sketch that is NOT on the ground plane, at top level, adopts
 * that sketch's plane and targets it directly (`SketchTarget.existing`)
 * instead of the shared per-plane ground cache. See the sketch-planes design
 * §1/§4 and `drawPlane.ts`/`sketchGesture.ts`.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import { makeSketchPlaneCache, type SketchPlaneCache } from './sketchGesture'
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

/** A tilted sketch: the y=0 plane, normal -Y (a vertical "wall" sketch). */
const TILTED_SKETCH = 55n
const TILTED_PLANE = new Float64Array([0, 0, 0, 0, -1, 0])
const GROUND_PLANE_ARR = new Float64Array([0, 0, 0, 0, 0, 1])

type SegmentCall = { sketch: bigint; a: [number, number, number]; b: [number, number, number] }

function makeWasmScene(opts: {
  /** `pick_sketch` result for every ray — a fixed handle (hit) or undefined (miss). */
  sketchPick?: bigint
  /** `sketch_plane` per handle; mutate to simulate a sketch vanishing mid-gesture. */
  planes?: Map<bigint, Float64Array | undefined>
} = {}) {
  const planes = opts.planes ?? new Map<bigint, Float64Array | undefined>([[TILTED_SKETCH, TILTED_PLANE]])
  const segmentCalls: SegmentCall[] = []
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
    sketch_add_segment: vi.fn(
      (sketch: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
        segmentCalls.push({ sketch, a: [ax, ay, az], b: [bx, by, bz] })
        const rc = nextRegionsCreated
        nextRegionsCreated = []
        return {
          new_edges: () => new BigUint64Array([]),
          regions_created: () => new BigUint64Array(rc),
          regions_removed: () => new BigUint64Array([]),
          free: vi.fn(),
        }
      },
    ),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  return {
    scene: scene as unknown as WasmScene,
    segmentCalls,
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
  const tool = new LineTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, cache)
  return { tool, onCommit, onToast, onFaceImprint, onMeasurement, cache }
}

describe('LineTool — sketch mode (drawing on a hovered non-ground sketch)', () => {
  it('hovering a rotated sketch anchors in sketch mode with its plane', () => {
    const { scene, segmentCalls } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 5 }), RAY)

    expect(scene.pick_sketch).toHaveBeenCalled()
    // Sketch mode targets the EXISTING handle — no ground sketch minted, and
    // the segment lands on the tilted sketch (asserted at commit, since a
    // ground-mode first click looks identical until something is committed).
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(segmentCalls.map((c) => c.sketch)).toEqual([TILTED_SKETCH])
  })

  it('a GRAZING pick the ray does not land on draws on the ground, not the tilted sketch', () => {
    // The regression behind the Follow Me L-path spec: `pick_sketch`'s cone
    // measures perpendicular distance to the ray, so a standing sketch seen
    // near edge-on "hits" for a click aimed at the ground half a metre away.
    // `rayLandsOnSketch` (drawPlane.ts) rejects it — the ray pierces y = 0
    // at (1, 0, -20), nowhere near SKETCH_LINES.
    const grazing: Ray = { origin: [1, 5, 0], direction: [0, -1, -4] }
    const { scene, segmentCalls } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), grazing)
    tool.onPointerDown(makeSnap({ x: 3, y: 2, z: 0 }), grazing)

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(segmentCalls).toHaveLength(1)
    expect(segmentCalls[0].sketch).not.toBe(TILTED_SKETCH)
    // Ground mode's z=0 fast path, not the tilted plane's basis math.
    expect(segmentCalls[0].a).toEqual([1, 1, 0])
    expect(segmentCalls[0].b).toEqual([3, 2, 0])
  })

  it('segments commit into the hovered sketch with on-plane world coordinates (z not forced to 0)', () => {
    const { scene, segmentCalls } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 2 }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 5 }), RAY)

    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(segmentCalls).toHaveLength(1)
    expect(segmentCalls[0].sketch).toBe(TILTED_SKETCH)
    expect(segmentCalls[0].a).toEqual([1, 0, 2])
    expect(segmentCalls[0].b).toEqual([3, 0, 5])
    // NOT the ground-mode z=0 fast path — the committed z is exactly the
    // snapped z, never zeroed.
    expect(segmentCalls[0].a[2]).not.toBe(0)
    expect(segmentCalls[0].b[2]).not.toBe(0)
    expect(onCommit).toHaveBeenCalledWith(TILTED_SKETCH)
  })

  it('a region-closing commit fires onCommit and ends the chain', () => {
    const { scene, setNextRegionsCreated } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0 }), RAY)
    setNextRegionsCreated([77n])
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 2 }), RAY)

    expect(onCommit).toHaveBeenCalledTimes(2) // fires every commit, closing or not
    expect(tool.capturingInput()).toBe(false) // the chain ended (region closed)
  })

  it('a stale hovered sketch mid-gesture toasts and does NOT create a ground sketch', () => {
    const { scene, planes } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // anchors sketch mode
    planes.delete(TILTED_SKETCH) // the sketch vanished (e.g. its creating gesture was undone)

    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0 }), RAY) // attempted commit

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled() // never a silent retarget
    expect(scene.sketch_add_segment).not.toHaveBeenCalled()
  })

  it('ground drawing (no sketch under the cursor) still shares the per-plane cache across tool instances', () => {
    const { scene, segmentCalls } = makeWasmScene({ sketchPick: undefined })
    const cache = makeSketchPlaneCache()
    const { tool: line1 } = makeTool(scene, cache)

    line1.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    line1.onPointerDown(makeSnap({ x: 1, y: 0 }), RAY)

    // A fresh tool instance (as after a tool switch) sharing the same cache
    // commits into the SAME ground sketch.
    const { tool: line2 } = makeTool(scene, cache)
    line2.onPointerDown(makeSnap({ x: 5, y: 5 }), RAY)
    line2.onPointerDown(makeSnap({ x: 6, y: 5 }), RAY)

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(new Set(segmentCalls.map((s) => s.sketch)).size).toBe(1)
    // The ground fast path: committed z is exactly 0.
    expect(segmentCalls[0].a[2]).toBe(0)
    expect(segmentCalls[0].b[2]).toBe(0)
  })
})
