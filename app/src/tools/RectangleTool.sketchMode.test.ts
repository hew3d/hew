/**
 * RectangleTool — sketch mode (sketches on any plane, Phase 2 wave A):
 * hovering a committed sketch that is NOT on the ground plane, at top
 * level, adopts that sketch's plane and targets it directly
 * (`SketchTarget.existing`) instead of the shared per-plane ground cache.
 * See the sketch-planes design §1/§4 and `drawPlane.ts`/`sketchGesture.ts`.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { RectangleTool } from './RectangleTool'
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

/** A tilted sketch: the y=0 plane, normal -Y (a vertical "wall" sketch).
 *  `facePlaneBasis([0,-1,0])` resolves to u=[0,0,1], v=[-1,0,0] — worked out
 *  once here so the expected commit corners below are exact, not approximate. */
const TILTED_SKETCH = 55n
const TILTED_PLANE = new Float64Array([0, 0, 0, 0, -1, 0])
const GROUND_PLANE_ARR = new Float64Array([0, 0, 0, 0, 0, 1])

type SegmentCall = { sketch: bigint; a: [number, number, number]; b: [number, number, number] }

function makeWasmScene(opts: {
  sketchPick?: bigint
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
        return {
          new_edges: () => new BigUint64Array([]),
          regions_created: () => new BigUint64Array(rc),
          regions_removed: () => new BigUint64Array([]),
          free: vi.fn(),
        }
      },
    ),
    split_face_inner: vi.fn(() => 99n),
  }
  return {
    scene: scene as unknown as WasmScene,
    segmentCalls,
    /** Sets the regions_created() result for every segment call FROM NOW ON
     *  (RectangleTool reports the LAST segment's regions_created, so setting
     *  this before the commit and never resetting it is enough). */
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
  const tool = new RectangleTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, cache)
  return { tool, onCommit, onToast, onFaceImprint, onMeasurement, cache }
}

describe('RectangleTool — sketch mode (drawing on a hovered non-ground sketch)', () => {
  it('hovering a rotated sketch anchors in sketch mode with its plane', () => {
    const { scene } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2 }), RAY)

    expect(tool.capturingInput()).toBe(true)
    expect(scene.pick_sketch).toHaveBeenCalled()
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

    tool.onPointerDown(makeSnap({ x: 0, y: 1, z: 0 }), grazing) // anchor
    tool.onPointerDown(makeSnap({ x: 3, y: 2, z: 0 }), grazing) // opposite corner — commits

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(scene.sketch_begin_gesture).not.toHaveBeenCalledWith(TILTED_SKETCH)
  })

  it('the second click commits four segments into the hovered sketch with on-plane world coordinates, and a closing commit fires onCommit', () => {
    const { scene, segmentCalls, setNextRegionsCreated } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onCommit } = makeTool(scene)

    setNextRegionsCreated([123n]) // the loop always closes — report it on every segment
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2 }), RAY) // anchor
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 5 }), RAY) // opposite corner — commits

    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(segmentCalls).toHaveLength(4)
    expect(segmentCalls.every((s) => s.sketch === TILTED_SKETCH)).toBe(true)
    // Exact corners on the y=0 plane (u=[0,0,1], v=[-1,0,0] for normal
    // [0,-1,0] — see the module-level comment; `faceRectangleCorners`
    // reverses winding when du/dv have opposite signs, which they do here):
    // (0,0,2) -> (3,0,2) -> (3,0,5) -> (0,0,5) -> back to (0,0,2). Every y is
    // exactly 0 (on-plane) and z varies between 2 and 5 — NOT the
    // ground-mode z=0 fast path.
    expect(segmentCalls).toEqual([
      { sketch: TILTED_SKETCH, a: [0, 0, 2], b: [3, 0, 2] },
      { sketch: TILTED_SKETCH, a: [3, 0, 2], b: [3, 0, 5] },
      { sketch: TILTED_SKETCH, a: [3, 0, 5], b: [0, 0, 5] },
      { sketch: TILTED_SKETCH, a: [0, 0, 5], b: [0, 0, 2] },
    ])
    expect(onCommit).toHaveBeenCalledWith({ sketchHandle: TILTED_SKETCH, regionsCreated: [123n] })
  })

  it('a stale hovered sketch mid-gesture toasts and does NOT create a ground sketch or commit any segment', () => {
    const { scene, planes } = makeWasmScene({ sketchPick: TILTED_SKETCH })
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2 }), RAY) // anchors sketch mode
    planes.delete(TILTED_SKETCH) // the sketch vanished mid-gesture

    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 5 }), RAY) // attempted commit

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_add_segment).not.toHaveBeenCalled()
  })

  it('ground drawing (no sketch under the cursor) still shares the per-plane cache across tool instances', () => {
    const { scene, segmentCalls } = makeWasmScene({ sketchPick: undefined })
    const cache = makeSketchPlaneCache()
    const { tool: rect1 } = makeTool(scene, cache)

    rect1.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    rect1.onPointerDown(makeSnap({ x: 1, y: 1 }), RAY)

    const { tool: rect2 } = makeTool(scene, cache) // fresh instance, same cache
    rect2.onPointerDown(makeSnap({ x: 5, y: 5 }), RAY)
    rect2.onPointerDown(makeSnap({ x: 6, y: 6 }), RAY)

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(new Set(segmentCalls.map((s) => s.sketch)).size).toBe(1)
    // The ground fast path: every committed z is exactly 0.
    expect(segmentCalls.every((s) => s.a[2] === 0 && s.b[2] === 0)).toBe(true)
  })
})
