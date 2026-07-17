import { describe, it, expect, vi } from 'vitest'
import { runSketchGesture, type SketchHandleCache } from './sketchGesture'
import type { Scene as WasmScene } from '../wasm/loader'

function makeCache(): SketchHandleCache & { value: bigint | null } {
  const box = { value: null as bigint | null }
  return {
    value: box.value,
    get: () => box.value,
    set: (h: bigint) => { box.value = h },
  }
}

/** `sketch_plane` shape for the ground plane: origin point, +Z normal. */
const GROUND = new Float64Array([0, 0, 0, 0, 0, 1])
/** A sketch stood upright (rotated 90° about X): y = 0 plane, −Y normal. */
const UPRIGHT = new Float64Array([0, 0, 0, 0, -1, 0])

describe('runSketchGesture', () => {
  it('mints a sketch when the cache is empty, brackets the body, and passes its return value through', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
      sketch_plane: vi.fn(() => GROUND),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeCache()

    const result = runSketchGesture(scene, cache, (sketch) => {
      expect(sketch).toBe(1n)
      return 'ok'
    })

    expect(result).toBe('ok')
    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    expect(scene.sketch_plane).not.toHaveBeenCalled() // nothing cached to check
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(1n)
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(1n)
  })

  it('reuses a cached handle whose sketch still lies on the ground plane', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
      sketch_plane: vi.fn(() => GROUND),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeCache()
    cache.set(7n)

    runSketchGesture(scene, cache, () => {})

    expect(scene.sketch_plane).toHaveBeenCalledWith(7n)
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(7n)
  })

  it('treats a flipped-but-coincident plane (−Z normal through the origin) as still ground', () => {
    // Orientation-free set test: every ground-tool point (z = 0) still lands
    // on a plane that merely faces the other way.
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
      sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, -1])),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeCache()
    cache.set(7n)

    runSketchGesture(scene, cache, () => {})

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
    const cache = makeCache()
    cache.set(7n)

    const seen: bigint[] = []
    runSketchGesture(scene, cache, (sketch) => { seen.push(sketch) })

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1)
    // The stale handle never even opens a gesture — no failure-driven retry.
    expect(scene.sketch_begin_gesture).toHaveBeenCalledTimes(1)
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(8n)
    expect(seen).toEqual([8n])
    expect(cache.get()).toBe(8n)
  })

  it('pre-checks a cached sketch that left the ground plane and retargets a fresh one up front — the body runs exactly once', () => {
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
    const cache = makeCache()
    cache.set(7n) // live, but no longer on the ground plane

    const seen: bigint[] = []
    const result = runSketchGesture(scene, cache, (sketch) => {
      seen.push(sketch)
      return 'drawn'
    })

    expect(result).toBe('drawn')
    expect(seen).toEqual([8n]) // never [7n, …] — nothing touches the tilted sketch
    expect(cache.get()).toBe(8n)
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
    const cache = makeCache()
    cache.set(1n)

    expect(() => runSketchGesture(scene, cache, () => {})).toThrow('SketchGestureAlreadyOpen')
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
    const cache = makeCache()

    expect(() =>
      runSketchGesture(scene, cache, () => {
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
    const cache = makeCache()
    cache.set(7n) // cached AND verified still on the ground plane

    const kernelRefusal: unknown = "PointOffPlane: point isn't on the sketch plane"
    let bodyCalls = 0
    let caught: unknown = null
    try {
      runSketchGesture(scene, cache, () => {
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
})
