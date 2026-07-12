import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { CircleTool } from './CircleTool'
import { segmentsPerTurn } from './arcMath'
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

/**
 * Minimal WasmScene stub — only the members CircleTool calls.
 */
function makeWasmScene(opts: {
  pick?: ReturnType<typeof makePick>
  facePlane?: [number, number, number, number, number, number]
  faceNormal?: [number, number, number]
  addSegmentThrows?: boolean
  splitFaceThrows?: boolean
  /** Make the FIRST `sketch_begin_gesture` call throw (stale cached handle),
   *  as if the sketch's creating gesture had been undone since caching. */
  beginGestureThrowsOnce?: boolean
} = {}): WasmScene {
  let sketchCounter = 41n
  let beginGestureFailuresLeft = opts.beginGestureThrowsOnce ? 1 : 0
  return {
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      return sketchCounter
    }),
    sketch_begin_gesture: vi.fn(() => {
      if (beginGestureFailuresLeft > 0) {
        beginGestureFailuresLeft -= 1
        throw new Error('UnknownSketch: stale or hidden handle')
      }
    }),
    sketch_end_gesture: vi.fn(),
    sketch_begin_curve: vi.fn(() => 91n),
    sketch_begin_curve_with: vi.fn(() => 91n),
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
    face_normal: vi.fn(() => new Float64Array(opts.faceNormal ?? [0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array(opts.facePlane ?? [0, 0, 0, 0, 0, 1])),
    split_face_inner: vi.fn(() => {
      if (opts.splitFaceThrows) throw new Error('LoopSelfIntersects: edges cross')
      return 99n
    }),
    split_face_inner_with_curve: vi.fn(() => {
      if (opts.splitFaceThrows) throw new Error('LoopSelfIntersects: edges cross')
      return 99n
    }),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new CircleTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, preview, onCommit, onToast, onFaceImprint, onMeasurement }
}

describe('CircleTool — ground mode', () => {
  it('two clicks (center, rim) commit N chained segments and call onCommit', () => {
    const scene = makeWasmScene()
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0 }), RAY) // rim, radius 3

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    // Adaptive facet count (true-curves §6): radius 3 caps at 96 per turn.
    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(segmentsPerTurn(3))
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith({ sketchHandle: 42n, regionsCreated: [] })
    expect(onToast).not.toHaveBeenCalled()
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
    expect(calls).toHaveLength(segmentsPerTurn(5))
    // First call's "p" (args 1-3) is vertex 0; last call's "q" (args 4-6) must match exactly.
    const firstP = [calls[0][1], calls[0][2], calls[0][3]]
    const lastQ = [calls[calls.length - 1][4], calls[calls.length - 1][5], calls[calls.length - 1][6]]
    expect(lastQ).toEqual(firstP)
    // Vertex 0 is exactly the rim point (5, 0, 0).
    expect(firstP).toEqual([5, 0, 0])
  })

  it('a degenerate (zero-radius) second click is skipped — no segments, no commit', () => {
    const scene = makeWasmScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // center
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // same point — degenerate

    expect(scene.sketch_add_segment).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    // Still anchored (first click stands) — capturingInput stays true.
    expect(tool.capturingInput()).toBe(true)
  })

  it('reuses the cached sketch handle across multiple circles', () => {
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

  it('a stale cached sketch handle (begin_gesture throws once) recovers by minting a fresh sketch and retrying', () => {
    const scene = makeWasmScene({ beginGestureThrowsOnce: true })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0 }), RAY) // rim — commits

    // begin_ground_sketch is called twice: once lazily, once on stale-handle retry.
    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(2)
    expect(scene.sketch_begin_gesture).toHaveBeenCalledTimes(2)
    // The retry used the SECOND (fresh) handle for the actual commit.
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith({ sketchHandle: 43n, regionsCreated: [] })
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(43n)
    expect(onToast).not.toHaveBeenCalled()
  })
})

describe('CircleTool — typed VCB radius entry', () => {
  it('typing a radius and pressing Enter commits an exact-radius circle', () => {
    const scene = makeWasmScene()
    const { tool, onCommit, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center
    expect(tool.capturingInput()).toBe(true)

    tool.onKey(makeKeyEvent('5'))
    expect(onMeasurement).toHaveBeenCalled()

    tool.onKey(makeKeyEvent('Enter'))

    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(segmentsPerTurn(5))
    expect(onCommit).toHaveBeenCalledTimes(1)
    // Default direction (+X) since cursor hasn't moved: vertex 0 should be (5, 0, 0).
    const calls = (scene.sketch_add_segment as ReturnType<typeof vi.fn>).mock.calls
    const firstP = [calls[0][1], calls[0][2], calls[0][3]]
    expect(firstP[0]).toBeCloseTo(5)
    expect(firstP[1]).toBeCloseTo(0)
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
})

describe('CircleTool — face mode', () => {
  it('two clicks on an entered object face call split_face_inner_with_curve carrying the circle', () => {
    const pick = makePick(7n, 3n)
    const scene = makeWasmScene({ pick, faceNormal: [0, 0, 1], facePlane: [0, 0, 0, 0, 0, 1] })
    const { tool, onFaceImprint, onToast } = makeTool(scene)
    tool.setActiveContext(7n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY) // center on face
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), { origin: [3, 0, 5], direction: [0, 0, -1] }) // rim click

    // Plain imprint is NOT used — the identity-carrying variant is.
    expect(scene.split_face_inner).not.toHaveBeenCalled()
    expect(scene.split_face_inner_with_curve).toHaveBeenCalledTimes(1)
    const callArgs = (scene.split_face_inner_with_curve as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(callArgs[0]).toBe(7n)
    expect(callArgs[1]).toBe(3n)
    const loopPts = callArgs[2] as Float64Array
    expect(loopPts.length).toBe(segmentsPerTurn(3) * 3)
    // center (0,0,0) and radius 3 travel with the imprint.
    const centerArg = callArgs[3] as Float64Array
    expect(Array.from(centerArg)).toEqual([0, 0, 0])
    expect(callArgs[4]).toBeCloseTo(3)
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
    expect(onToast).not.toHaveBeenCalled()
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

describe('CircleTool — cancel', () => {
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
})

describe('CircleTool — capturingInput scoping', () => {
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
