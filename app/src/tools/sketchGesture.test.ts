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

describe('runSketchGesture', () => {
  it('mints a sketch when the cache is empty, brackets the body, and passes its return value through', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
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
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(1n)
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(1n)
  })

  it('reuses an already-cached handle without minting a new sketch', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeCache()
    cache.set(7n)

    runSketchGesture(scene, cache, () => {})

    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_begin_gesture).toHaveBeenCalledWith(7n)
  })

  it('recovers from a stale cached handle: begin_gesture throwing once mints a fresh sketch and retries', () => {
    let failuresLeft = 1
    let nextHandle = 7n
    const scene = {
      begin_ground_sketch: vi.fn(() => ++nextHandle),
      sketch_begin_gesture: vi.fn(() => {
        if (failuresLeft > 0) {
          failuresLeft -= 1
          throw new Error('UnknownSketch: stale or hidden handle')
        }
      }),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeCache()
    cache.set(7n) // stale — its creating gesture was undone

    const seen: bigint[] = []
    runSketchGesture(scene, cache, (sketch) => { seen.push(sketch) })

    expect(scene.begin_ground_sketch).toHaveBeenCalledTimes(1) // one fresh mint on retry
    expect(scene.sketch_begin_gesture).toHaveBeenCalledTimes(2) // the failed try + the retry
    expect(seen).toEqual([8n])
    expect(scene.sketch_end_gesture).toHaveBeenCalledWith(8n)
    expect(cache.get()).toBe(8n) // the cache now holds the fresh handle
  })

  it('propagates a second (genuine) failure instead of looping forever', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 2n),
      sketch_begin_gesture: vi.fn(() => {
        throw new Error('SketchGestureAlreadyOpen: gestures never nest')
      }),
      sketch_end_gesture: vi.fn(),
    } as unknown as WasmScene
    const cache = makeCache()
    cache.set(1n)

    expect(() => runSketchGesture(scene, cache, () => {})).toThrow('SketchGestureAlreadyOpen')
    // end_gesture is never reached — the bracket never successfully opened.
    expect(scene.sketch_end_gesture).not.toHaveBeenCalled()
  })

  it('still closes the gesture (recording whatever succeeded) when the body throws mid-commit', () => {
    const scene = {
      begin_ground_sketch: vi.fn(() => 1n),
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
})
