import { describe, it, expect, vi } from 'vitest'
import { runSketchGesture, makeSketchPlaneCache, type SketchTarget } from './sketchGesture'
import { groundDrawPlane, planeKey } from './drawPlane'
import type { DrawPlane } from './drawPlane'
import type { Scene as WasmScene } from '../wasm/loader'

/** `sketch_plane` shape for the ground plane: origin point, +Z normal. */
const GROUND = new Float64Array([0, 0, 0, 0, 0, 1])
/** A sketch stood upright (rotated 90° about X): y = 0 plane, −Y normal. */
const UPRIGHT = new Float64Array([0, 0, 0, 0, -1, 0])

const GROUND_PLANE: SketchTarget = { kind: 'plane', plane: groundDrawPlane() }
const GROUND_KEY = planeKey(groundDrawPlane())

/** A non-ground plane target — the y=0, -Y-normal plane (an idle-locked
 *  plane, Phase 3; reachable only via a draw tool's idle plane lock —
 *  sketch mode instead targets an `existing` handle — but exercised
 *  directly here since `runSketchGesture` owns the minting decision). */
const TILTED_PLANE: DrawPlane = { origin: [0, 0, 0], normal: [0, -1, 0], u: [1, 0, 0], v: [0, 0, -1], ground: false }

describe('runSketchGesture — plane targets', () => {
  it('mints a sketch when the cache is empty, brackets the body, and passes its return value through', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
      sketch_plane: vi.fn(() => GROUND),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()

    const result = runSketchGesture(scene, cache, GROUND_PLANE, (sketch) => {
      expect(sketch).toBe(1n)
      return 'ok'
    })

    expect(result).toBe('ok')
    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(scene.sketch_plane).not.toHaveBeenCalled() // nothing cached to check
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(1n)
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(1n)
  })

  it('reuses a cached handle whose sketch still lies on the target plane', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
      sketch_plane: vi.fn(() => GROUND),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()
    cache.set(GROUND_KEY, 7n)

    runSketchGesture(scene, cache, GROUND_PLANE, () => {})

    expect(scene.sketch_plane).toHaveBeenCalledWith(7n)
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(7n)
  })

  it('treats a flipped-but-coincident plane (−Z normal through the origin) as still on the ground plane', () => {
    // Orientation-free set test: every ground-tool point (z = 0) still lands
    // on a plane that merely faces the other way.
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
      sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, -1])),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()
    cache.set(GROUND_KEY, 7n)

    runSketchGesture(scene, cache, GROUND_PLANE, () => {})

    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(7n)
  })

  it('pre-checks a stale cached handle (sketch_plane undefined) and mints a fresh sketch BEFORE the gesture opens', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 8n),
      sketch_plane: vi.fn(() => undefined), // creating gesture was undone
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()
    cache.set(GROUND_KEY, 7n)

    const seen: bigint[] = []
    runSketchGesture(scene, cache, GROUND_PLANE, (sketch) => { seen.push(sketch) })

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    // The stale handle never even opens a gesture — no failure-driven retry.
    expect(scene.sketch_begin_gesture).toHaveBeenCalledTimes(1)
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(8n)
    expect(seen).toEqual([8n])
    expect(cache.get(GROUND_KEY)).toBe(8n)
  })

  it('pre-checks a cached sketch that left the target plane and retargets a fresh one up front — the body runs exactly once', () => {
    // The cached sketch was rotated upright since caching: its handle is
    // still LIVE, so only the plane read can tell. The old failure-driven
    // retry waited for the body to throw PointOffPlane — but a multi-segment
    // body (rectangle/circle/arc) can succeed for segments that happen to
    // lie on the tilted plane, stranding real edges there plus a spurious
    // undo step before any recovery. The pre-check decides before anything
    // is submitted.
    const scene = {
      begin_ground_sketch: vi.fn(() => 8n),
      sketch_plane: vi.fn(() => UPRIGHT),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()
    cache.set(GROUND_KEY, 7n) // live, but no longer on the ground plane

    const seen: bigint[] = []
    const result = runSketchGesture(scene, cache, GROUND_PLANE, (sketch) => {
      seen.push(sketch)
      return 'drawn'
    })

    expect(result).toBe('drawn')
    expect(seen).toEqual([8n]) // never [7n, …] — nothing touches the tilted sketch
    expect(cache.get(GROUND_KEY)).toBe(8n)
    expect(scene.sketch_begin_gesture).toHaveBeenCalledTimes(1)
    expect(scene.sketch_end_gesture).toHaveBeenCalledTimes(1)
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(8n)
  })

  it('propagates a genuine begin_gesture failure instead of retrying', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 2n),
      sketch_plane: vi.fn(() => GROUND),
      sketch_begin_gesture: vi.fn(() => {
        throw new Error('SketchGestureAlreadyOpen: gestures never nest')
      }),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()
    cache.set(GROUND_KEY, 1n)

    expect(() => runSketchGesture(scene, cache, GROUND_PLANE, () => {})).toThrow('SketchGestureAlreadyOpen')
    // No junk sketch is minted for a genuine failure, and end_gesture is
    // never reached — the bracket never successfully opened.
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_end_gesture).not.toHaveBeenCalled()
  })

  it('still closes the gesture (recording whatever succeeded) when the body throws mid-commit', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
      sketch_plane: vi.fn(() => GROUND),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()

    expect(() =>
      runSketchGesture(scene, cache, GROUND_PLANE, () => {
        throw new Error('PathNotSimple: edges cross')
      }),
    ).toThrow('PathNotSimple')

    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(1n)
  })

  it('does NOT swallow PointOffPlane from the body — with the pre-check it is always a genuine refusal', () => {
    // Kernel errors cross the WASM boundary as plain strings ("CODE: …"),
    // not Error instances — model that faithfully so a future failure-driven
    // handler can't sneak back in on an instanceof check.
    const scene = {
      begin_ground_sketch: vi.fn(() => 8n),
      sketch_plane: vi.fn(() => GROUND),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()
    cache.set(GROUND_KEY, 7n) // cached AND verified still on the ground plane

    const kernelRefusal: unknown = "PointOffPlane: point isn't on the sketch plane"
    let bodyCalls = 0
    let caught: unknown = null
    try {
      runSketchGesture(scene, cache, GROUND_PLANE, () => {
        bodyCalls++
        throw kernelRefusal
      })
    } catch (e) {
      caught = e
    }

    expect(caught).toBe(kernelRefusal) // propagated verbatim
    expect(bodyCalls).toBe(1) // no re-run against a freshly minted sketch
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(7n) // bracket closed
  })

  it('a non-ground plane target with no cached handle mints via begin_sketch_on_plane (Phase 3)', () => {
    const scene = {
      begin_ground_sketch: vi.fn(),
      begin_sketch_on_plane: vi.fn(() => 9n),
      sketch_plane: vi.fn(() => undefined),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()

    const seen: bigint[] = []
    runSketchGesture(scene, cache, { kind: 'plane', plane: TILTED_PLANE }, (sketch) => { seen.push(sketch) })

    expect(scene.begin_sketch_on_plane).toHaveBeenCalledWith(0, 0, 0, 0, -1, 0)
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(seen).toEqual([9n])
    expect(cache.get(planeKey(TILTED_PLANE))).toBe(9n)
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(9n)
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(9n)
  })

  it('a cached non-ground handle still on its plane is reused — begin_sketch_on_plane is not called again', () => {
    const scene = {
      begin_ground_sketch: vi.fn(),
      begin_sketch_on_plane: vi.fn(() => 9n),
      sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, -1, 0])), // live, on TILTED_PLANE
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()
    cache.set(planeKey(TILTED_PLANE), 3n)

    runSketchGesture(scene, cache, { kind: 'plane', plane: TILTED_PLANE }, () => {})

    expect(scene.begin_sketch_on_plane).not.toHaveBeenCalled()
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(3n)
  })

  it('two DrawPlane instances that key the same share one cached handle (one sketch per plane)', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
      sketch_plane: vi.fn(() => GROUND),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()

    // Ground plane, described through two different (but equal-keying) points.
    const planeA: DrawPlane = groundDrawPlane()
    const planeB: DrawPlane = { origin: [5, -3, 0], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0], ground: false }

    const seen: bigint[] = []
    runSketchGesture(scene, cache, { kind: 'plane', plane: planeA }, (s) => seen.push(s))
    runSketchGesture(scene, cache, { kind: 'plane', plane: planeB }, (s) => seen.push(s))

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(seen).toEqual([1n, 1n])
  })
})

describe('runSketchGesture — existing (sketch-mode) targets', () => {
  it('uses the target handle as-is when its sketch is live — no plane lookup, no minting', () => {
    const scene = {
      begin_ground_sketch: vi.fn(),
      sketch_plane: vi.fn(() => UPRIGHT), // live, on some arbitrary non-ground plane
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()

    const result = runSketchGesture(scene, cache, { kind: 'existing', handle: 42n }, (sketch) => {
      expect(sketch).toBe(42n)
      return 'ok'
    })

    expect(result).toBe('ok')
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(42n)
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(42n)
  })

  it('a vanished existing target throws an UnknownSketch-prefixed error and never opens a gesture or mints a ground sketch', () => {
    const scene = {
      begin_ground_sketch: vi.fn(),
      sketch_plane: vi.fn(() => undefined),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeSketchPlaneCache()

    expect(() =>
      runSketchGesture(scene, cache, { kind: 'existing', handle: 42n }, () => {}),
    ).toThrow(/^UnknownSketch/)
    expect(scene.sketch_begin_gesture).not.toHaveBeenCalled()
    expect(scene.sketch_end_gesture).not.toHaveBeenCalled()
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled() // never a silent retarget
  })
})

describe('SketchPlaneCache', () => {
  it('get/set/clear round-trip per key, independent of other keys', () => {
    const cache = makeSketchPlaneCache()
    expect(cache.get('a')).toBeNull()
    cache.set('a', 1n)
    cache.set('b', 2n)
    expect(cache.get('a')).toBe(1n)
    expect(cache.get('b')).toBe(2n)
    cache.set('a', null)
    expect(cache.get('a')).toBeNull()
    expect(cache.get('b')).toBe(2n) // untouched
    cache.clear()
    expect(cache.get('b')).toBeNull()
  })
})
