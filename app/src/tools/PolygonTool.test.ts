import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { PolygonTool, DEFAULT_POLYGON_SIDES, MIN_POLYGON_SIDES, MAX_POLYGON_SIDES } from './PolygonTool'
import { makeSketchPlaneCache } from './sketchGesture'
import { groundDrawPlane, planeKey } from './drawPlane'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

// A ray straight down the -Z axis from above the origin (tuple-shaped, as the
// real Viewport supplies — the tool indexes ray.origin[0..2]).
const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeKeyEvent(key: string): KeyboardEvent {
  return { key, preventDefault: () => {} } as unknown as KeyboardEvent
}

/** A fake `FacePickJs` returning the seeded handles. */
function makePick(object: bigint, face: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => undefined,
    free: vi.fn(),
  }
}

/** Minimal WasmScene stub — only the members PolygonTool calls. */
function makeWasmScene(opts: {
  pick?: ReturnType<typeof makePick>
  facePlane?: [number, number, number, number, number, number]
  faceNormal?: [number, number, number]
  addSegmentThrows?: boolean
  splitFaceThrows?: boolean
  /** Handles whose sketch has gone stale/hidden — `sketch_plane` reads
   *  `undefined` for them, so `runSketchGesture`'s pre-check retargets a
   *  fresh sketch (as after undoing the sketch's creating gesture). */
  staleSketchHandles?: bigint[]
} = {}): WasmScene {
  let sketchCounter = 41n
  return {
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      return sketchCounter
    }),
    begin_sketch_on_plane: vi.fn(() => {
      sketchCounter += 1n
      return sketchCounter
    }),
    // Every non-stale sketch lies on the ground plane (origin point, +Z).
    sketch_plane: vi.fn((sketch: bigint) =>
      (opts.staleSketchHandles ?? []).includes(sketch)
        ? undefined
        : new Float64Array([0, 0, 0, 0, 0, 1]),
    ),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    // A polygon's plane-mode commit is bracketed as ONE polygon chain,
    // carrying the drawn centre and circumradius — that is what makes its
    // centre selectable and inferable.
    sketch_begin_polygon_with: vi.fn(() => 7n),
    sketch_end_curve: vi.fn(),
    sketch_add_segment: vi.fn(() => {
      if (opts.addSegmentThrows) throw new Error('PathNotSimple: edges cross')
      return {
        new_edges: () => new BigUint64Array([]),
        regions_created: () => new BigUint64Array([]),
        regions_removed: () => new BigUint64Array([]),
        free: vi.fn(),
      }
    }),
    pick_face: vi.fn(() => opts.pick),
    pick_sketch: vi.fn(() => undefined), // no committed sketches in these fixtures
    face_normal: vi.fn(() => new Float64Array(opts.faceNormal ?? [0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array(opts.facePlane ?? [0, 0, 0, 0, 0, 1])),
    split_face_inner: vi.fn(() => {
      if (opts.splitFaceThrows) throw new Error('LoopSelfIntersects: edges cross')
      return 99n
    }),
    split_face_inner_with_curve: vi.fn(() => 99n),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const onSideCountChange = vi.fn()
  const tool = new PolygonTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, makeSketchPlaneCache(), onSideCountChange)
  return { tool, preview, onCommit, onToast, onFaceImprint, onMeasurement, onSideCountChange }
}

describe('PolygonTool — ground mode', () => {
  it('defaults to 6 sides', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    expect(tool.sideCount).toBe(DEFAULT_POLYGON_SIDES)
  })

  it('two clicks (center, rim) commit exactly N=6 chained plain segments and call onCommit', () => {
    const scene = makeWasmScene()
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0 }), RAY) // rim, radius 3

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(6)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith({ sketchHandle: 42n, regionsCreated: [] })
    expect(onToast).not.toHaveBeenCalled()
    // Face mode's curve-carrying imprint is a CircleTool thing; a polygon
    // never claims an analytic circle on a solid (design §4/§8).
    expect(scene.split_face_inner_with_curve).not.toHaveBeenCalled()
    // In plane mode the polygon IS one chain, opened as a POLYGON (not a
    // circle) with the drawn centre (0,0,0) and circumradius 3 — the record
    // that makes its centre snappable. Bracket closed exactly once.
    expect(scene.sketch_begin_polygon_with).toHaveBeenCalledTimes(1)
    expect(scene.sketch_begin_polygon_with).toHaveBeenCalledWith(42n, 0, 0, 0, 3)
    expect(scene.sketch_end_curve).toHaveBeenCalledTimes(1)
    // The whole N-segment commit is bracketed in exactly one gesture.
    expect(scene.sketch_begin_gesture).toHaveBeenCalledTimes(1)
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(42n)
    expect(scene.sketch_end_gesture).toHaveBeenCalledTimes(1)
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(42n)
  })

  it('the last segment closes back to the exact stored vertex 0 coordinates', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 5, y: 0, z: 0 }), RAY)

    const calls = (scene.sketch_add_segment as ReturnType<typeof vi.fn>).mock.calls
    expect(calls).toHaveLength(6)
    const firstP = [calls[0][1], calls[0][2], calls[0][3]]
    const lastQ = [calls[calls.length - 1][4], calls[calls.length - 1][5], calls[calls.length - 1][6]]
    expect(lastQ).toEqual(firstP)
    // Vertex 0 is exactly the rim point (5, 0, 0).
    expect(firstP).toEqual([5, 0, 0])
  })

  it('every committed vertex lies on the circle of the given radius (within tolerance)', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // center (1,1)
    tool.onPointerDown(makeSnap({ x: 5, y: 1, z: 0 }), RAY) // rim — radius 4

    const calls = (scene.sketch_add_segment as ReturnType<typeof vi.fn>).mock.calls
    for (const call of calls) {
      const p: [number, number, number] = [call[1], call[2], call[3]]
      const r = Math.hypot(p[0] - 1, p[1] - 1)
      expect(r).toBeCloseTo(4)
    }
  })

  it('a degenerate (zero-radius) second click is skipped — no segments, no commit, stays anchored', () => {
    const scene = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // center
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // same point — degenerate

    expect(scene.sketch_add_segment).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true)
  })

  it('reuses the cached sketch handle across multiple polygons', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 10, y: 10, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 12, y: 10, z: 0 }), RAY)

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
  })

  it('a refused commit (kernel throws) toasts and does not call onCommit', () => {
    const scene = makeWasmScene({ addSegmentThrows: true })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0 }), RAY)

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('a stale cached sketch handle (sketch_plane reads undefined) is retargeted onto a fresh sketch before the gesture opens', () => {
    const scene = makeWasmScene({ staleSketchHandles: [7n] })
    const preview = new THREE.Group()
    const onCommit = vi.fn()
    const onToast = vi.fn()
    const cache = makeSketchPlaneCache()
    cache.set(planeKey(groundDrawPlane()), 7n)
    const tool = new PolygonTool(scene, preview, onCommit, onToast, vi.fn(), vi.fn(), cache)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0 }), RAY) // rim — commits

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(scene.sketch_begin_gesture).toHaveBeenCalledTimes(1)
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(42n)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(42n)
    expect(onToast).not.toHaveBeenCalled()
  })
})

describe('PolygonTool — typed VCB: side count (Ns)', () => {
  it('8s sets the side count to 8, stays anchored (no commit)', () => {
    const scene = makeWasmScene()
    const { tool, onCommit, onSideCountChange } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    tool.onKey(makeKeyEvent('8'))
    tool.onKey(makeKeyEvent('s'))
    tool.onKey(makeKeyEvent('Enter'))

    expect(tool.sideCount).toBe(8)
    expect(onSideCountChange).toHaveBeenCalledWith(8)
    expect(onCommit).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true)
  })

  it('a subsequent commit uses the newly typed side count', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('8'))
    tool.onKey(makeKeyEvent('s'))
    tool.onKey(makeKeyEvent('Enter'))
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0 }), RAY) // rim — commits

    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(8)
  })

  it('2s is clamped to the minimum side count (3)', () => {
    const scene = makeWasmScene()
    const { tool, onSideCountChange } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('2'))
    tool.onKey(makeKeyEvent('s'))
    tool.onKey(makeKeyEvent('Enter'))

    expect(tool.sideCount).toBe(MIN_POLYGON_SIDES)
    expect(onSideCountChange).toHaveBeenCalledWith(MIN_POLYGON_SIDES)
  })

  it('a huge side count is clamped to the maximum', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    for (const ch of '999999') tool.onKey(makeKeyEvent(ch))
    tool.onKey(makeKeyEvent('s'))
    tool.onKey(makeKeyEvent('Enter'))

    expect(tool.sideCount).toBe(MAX_POLYGON_SIDES)
  })

  it('setSideCount (Viewport session persistence) clamps and does not fire OnSideCountChange', () => {
    const scene = makeWasmScene()
    const { tool, onSideCountChange } = makeTool(scene)

    tool.setSideCount(10)
    expect(tool.sideCount).toBe(10)
    tool.setSideCount(1)
    expect(tool.sideCount).toBe(MIN_POLYGON_SIDES)
    expect(onSideCountChange).not.toHaveBeenCalled()
  })

  it('the side count persists across multiple gestures on the same tool instance', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('8'))
    tool.onKey(makeKeyEvent('s'))
    tool.onKey(makeKeyEvent('Enter'))
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0 }), RAY) // commits first polygon

    ;(scene.sketch_add_segment as ReturnType<typeof vi.fn>).mockClear()

    tool.onPointerDown(makeSnap({ x: 10, y: 10, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 12, y: 10, z: 0 }), RAY) // second polygon, no re-typing

    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(8)
  })
})

describe('PolygonTool — typed VCB: circumradius entry', () => {
  it('typing a radius and pressing Enter commits an exact-circumradius polygon', () => {
    const scene = makeWasmScene()
    const { tool, onCommit, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    expect(tool.capturingInput()).toBe(true)

    tool.onKey(makeKeyEvent('5'))
    expect(onMeasurement).toHaveBeenCalled()

    tool.onKey(makeKeyEvent('Enter'))

    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(6)
    expect(onCommit).toHaveBeenCalledTimes(1)
    // Default direction (+X) since cursor hasn't moved: vertex 0 should be (5, 0, 0).
    const calls = (scene.sketch_add_segment as ReturnType<typeof vi.fn>).mock.calls
    const firstP = [calls[0][1], calls[0][2], calls[0][3]]
    expect(firstP[0]).toBeCloseTo(5)
    expect(firstP[1]).toBeCloseTo(0)
  })

  it('an explicit-unit radius (10mm) commits a circumradius of 0.01 m', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    for (const ch of '10mm') tool.onKey(makeKeyEvent(ch))
    tool.onKey(makeKeyEvent('Enter'))

    const calls = (scene.sketch_add_segment as ReturnType<typeof vi.fn>).mock.calls
    const firstP = [calls[0][1], calls[0][2], calls[0][3]]
    const r = Math.hypot(firstP[0], firstP[1])
    expect(r).toBeCloseTo(0.01)
  })

  it('typed radius follows the last rubber-band cursor direction', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    tool.onPointerMove(makeSnap({ x: 0, y: -1, z: 0 }), RAY) // cursor toward -Y

    tool.onKey(makeKeyEvent('2'))
    tool.onKey(makeKeyEvent('Enter'))

    const calls = (scene.sketch_add_segment as ReturnType<typeof vi.fn>).mock.calls
    const firstP = [calls[0][1], calls[0][2], calls[0][3]]
    expect(firstP[0]).toBeCloseTo(0)
    expect(firstP[1]).toBeCloseTo(-2)
  })

  it('Enter with an empty buffer does nothing (no commit)', () => {
    const scene = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('Enter'))

    expect(onCommit).not.toHaveBeenCalled()
  })

  it('a typed sub-tolerance radius (0) is a no-op that STAYS in the gesture — center preserved', () => {
    const scene = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    tool.onKey(makeKeyEvent('0'))
    tool.onKey(makeKeyEvent('Enter'))

    // Degenerate: no teardown, no commit.
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.sketch_add_segment).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true)

    // The placed center survived — a real second click still commits from it.
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0 }), RAY)
    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(6)
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('a typed negative radius commits its magnitude (abs), not a 180°-flipped polygon', () => {
    const scene = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    tool.onKey(makeKeyEvent('-'))
    tool.onKey(makeKeyEvent('5'))
    tool.onKey(makeKeyEvent('Enter'))

    expect(onCommit).toHaveBeenCalledTimes(1)
    const calls = (scene.sketch_add_segment as ReturnType<typeof vi.fn>).mock.calls
    const firstP = [calls[0][1], calls[0][2], calls[0][3]]
    // Radius 5 in the +X default direction — NOT flipped to (-5, 0, 0).
    expect(firstP[0]).toBeCloseTo(5)
    expect(firstP[1]).toBeCloseTo(0)
  })

  it('combines Ns and a typed radius in one gesture (8s, then 10mm)', () => {
    const scene = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('8'))
    tool.onKey(makeKeyEvent('s'))
    tool.onKey(makeKeyEvent('Enter'))
    for (const ch of '10mm') tool.onKey(makeKeyEvent(ch))
    tool.onKey(makeKeyEvent('Enter'))

    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(8)
    const calls = (scene.sketch_add_segment as ReturnType<typeof vi.fn>).mock.calls
    const firstP = [calls[0][1], calls[0][2], calls[0][3]]
    expect(Math.hypot(firstP[0], firstP[1])).toBeCloseTo(0.01)
  })
})

describe('PolygonTool — face mode', () => {
  it('two clicks on an entered object face call split_face_inner (no curve identity)', () => {
    const pick = makePick(7n, 3n)
    const scene = makeWasmScene({ pick, faceNormal: [0, 0, 1], facePlane: [0, 0, 0, 0, 0, 1] })
    const { tool, onFaceImprint, onToast } = makeTool(scene)
    tool.setActiveContext(7n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center on face
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), { origin: [3, 0, 5], direction: [0, 0, -1] }) // rim click

    expect(scene.split_face_inner).toHaveBeenCalledTimes(1)
    expect(scene.split_face_inner_with_curve).not.toHaveBeenCalled()
    const callArgs = (scene.split_face_inner as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(callArgs[0]).toBe(7n)
    expect(callArgs[1]).toBe(3n)
    const loopPts = callArgs[2] as Float64Array
    expect(loopPts.length).toBe(6 * 3) // default 6 sides
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a degenerate (zero-radius) second click on a face STAYS anchored — no imprint, center preserved', () => {
    const pick = makePick(7n, 3n)
    const scene = makeWasmScene({ pick, faceNormal: [0, 0, 1], facePlane: [0, 0, 0, 0, 0, 1] })
    const { tool, onFaceImprint } = makeTool(scene)
    tool.setActiveContext(7n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center on face
    expect(tool.capturingInput()).toBe(true)
    // Second click projects to the SAME plane point as the center → zero radius.
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    // No imprint, and — crucially — the center is not silently dropped.
    expect(scene.split_face_inner).not.toHaveBeenCalled()
    expect(onFaceImprint).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true)

    // A real second click from the preserved center still imprints.
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), { origin: [3, 0, 5], direction: [0, 0, -1] })
    expect(scene.split_face_inner).toHaveBeenCalledTimes(1)
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
  })

  it('a pick on a different object than the active context is ignored', () => {
    const pick = makePick(999n, 3n) // not the active context (7n)
    const scene = makeWasmScene({ pick })
    const { tool } = makeTool(scene)
    tool.setActiveContext(7n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    expect(tool.capturingInput()).toBe(false)
  })

  it('a refused split_face_inner toasts the kernel error and does not call onFaceImprint', () => {
    const pick = makePick(7n, 3n)
    const scene = makeWasmScene({ pick, splitFaceThrows: true })
    const { tool, onFaceImprint, onToast } = makeTool(scene)
    tool.setActiveContext(7n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), { origin: [3, 0, 5], direction: [0, 0, -1] })

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onFaceImprint).not.toHaveBeenCalled()
  })
})

describe('PolygonTool — cancel', () => {
  it('Escape after the first click cancels and clears the preview', () => {
    const scene = makeWasmScene()
    const { tool, preview } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onPointerMove(makeSnap({ x: 3, y: 0, z: 0 }), RAY)
    expect(preview.children.length).toBeGreaterThan(0)

    tool.onKey(makeKeyEvent('Escape'))

    expect(tool.capturingInput()).toBe(false)
    expect(preview.children).toHaveLength(0)
  })

  it('Escape before any click is a no-op cancel (idle stays idle)', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.capturingInput()).toBe(false)
  })

  it('cancel does not reset the side count', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    tool.onKey(makeKeyEvent('8'))
    tool.onKey(makeKeyEvent('s'))
    tool.onKey(makeKeyEvent('Enter'))
    tool.cancel()

    expect(tool.sideCount).toBe(8)
  })
})

describe('PolygonTool — capturingInput scoping', () => {
  it('is false when idle (so tool-switch shortcuts are not swallowed)', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    expect(tool.capturingInput()).toBe(false)
  })

  it('becomes true only after a center is anchored', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    expect(tool.capturingInput()).toBe(false)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)
    expect(tool.capturingInput()).toBe(true)
  })
})
