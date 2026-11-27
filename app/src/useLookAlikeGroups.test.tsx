/**
 * The look-alike read behind Edit ▸ Link identical copies and the Model
 * Health notice runs off the render path: after the document settles, once
 * per burst of changes, never while a component is open, and not for a very
 * large model.
 */
import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Scene as WasmScene } from './wasm/loader'
import { LOOK_ALIKE_MAX_OBJECTS, useLookAlikeGroups } from './useLookAlikeGroups'
import { tagged } from './test/taggedDispatch'

const frame = (x: number) => ({ origin: [x, 0, 0], x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1], extents: [1, 1, 1] })

function mockScene(count: number) {
  let ids = Array.from({ length: count }, (_, i) => BigInt(i + 1))
  const signatureCalls = vi.fn()
  const scene = {
    object_ids: () => BigUint64Array.from(ids),
    node_is_live: () => true,
    node_parent: () => undefined,
    object_public_ids: (a: BigUint64Array) => Array.from(a).map((n) => `obj_${n}`),
    api_connection_open: () => 1,
    api_connection_close: () => {},
    api_dispatch: (_c: number, raw: string) => {
      const req = JSON.parse(raw) as { id: number; method: string; params?: { id?: string } }
      if (req.method !== 'hew.query.signature') return tagged(JSON.stringify({ jsonrpc: '2.0', id: req.id, result: {} }))
      signatureCalls()
      const n = Number(String(req.params?.id).slice(4))
      return tagged(JSON.stringify({ jsonrpc: '2.0', id: req.id, result: { shape: 'A', mirror: 'B', symmetric: false, frame: frame(n) } }))
    },
  } as unknown as WasmScene
  return { scene, signatureCalls, setCount: (n: number) => (ids = Array.from({ length: n }, (_, i) => BigInt(i + 1))) }
}

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

describe('useLookAlikeGroups', () => {
  it('reads nothing during render and finds the groups once the document settles', () => {
    const m = mockScene(3)
    const { result } = renderHook(({ rev }) => useLookAlikeGroups(m.scene, rev, false), { initialProps: { rev: 0 } })
    expect(m.signatureCalls).not.toHaveBeenCalled()
    expect(result.current).toEqual([])
    act(() => vi.runAllTimers())
    expect(m.signatureCalls).toHaveBeenCalledTimes(3)
    expect(result.current).toHaveLength(1)
    expect(result.current[0].members).toHaveLength(3)
  })

  it('a burst of edits reads once, and the notice goes away when the look-alikes do', () => {
    const m = mockScene(2)
    const { result, rerender } = renderHook(({ rev }) => useLookAlikeGroups(m.scene, rev, false), { initialProps: { rev: 0 } })
    act(() => vi.runAllTimers())
    expect(result.current).toHaveLength(1)
    m.signatureCalls.mockClear()
    m.setCount(1)
    for (let rev = 1; rev <= 5; rev++) rerender({ rev })
    expect(m.signatureCalls).not.toHaveBeenCalled()
    act(() => vi.runAllTimers())
    expect(m.signatureCalls).toHaveBeenCalledTimes(1)
    expect(result.current).toEqual([])
  })

  it('offers nothing while a component is open, and reads nothing then', () => {
    const m = mockScene(2)
    const { result, rerender } = renderHook(({ off }) => useLookAlikeGroups(m.scene, 0, off), { initialProps: { off: false } })
    act(() => vi.runAllTimers())
    expect(result.current).toHaveLength(1)
    m.signatureCalls.mockClear()
    rerender({ off: true })
    expect(result.current).toEqual([])
    act(() => vi.runAllTimers())
    expect(m.signatureCalls).not.toHaveBeenCalled()
  })

  it('skips a very large model and a missing scene', () => {
    const m = mockScene(LOOK_ALIKE_MAX_OBJECTS + 1)
    const { result } = renderHook(() => useLookAlikeGroups(m.scene, 0, false))
    act(() => vi.runAllTimers())
    expect(m.signatureCalls).not.toHaveBeenCalled()
    expect(result.current).toEqual([])
    const none = renderHook(() => useLookAlikeGroups(null, 0, false))
    act(() => vi.runAllTimers())
    expect(none.result.current).toEqual([])
  })

  it('a different document never shows the last one\'s groups', () => {
    const a = mockScene(2)
    const b = mockScene(1)
    const { result, rerender } = renderHook(({ scene }) => useLookAlikeGroups(scene, 0, false), { initialProps: { scene: a.scene } })
    act(() => vi.runAllTimers())
    expect(result.current).toHaveLength(1)
    rerender({ scene: b.scene })
    expect(result.current).toEqual([])
  })
})
