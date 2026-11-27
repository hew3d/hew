/**
 * The compounds this window holds, against a kernel that closes only the
 * innermost one: whoever closes a compound first ends every compound above
 * it, and nothing this window did not open is ever closed.
 */

import { describe, expect, it, vi } from 'vitest'
import type { Scene } from '../wasm/loader'
import { setBusy } from './busyState'
import {
  abortCompound,
  beginCompound,
  endCompound,
  innermostForward,
  isForwardOpen,
  openCompoundCount,
  type OpenCompound,
} from './hostCompounds'

function kernel() {
  const stack: bigint[] = []
  let next = 1n
  const s = {
    stack,
    busy: vi.fn(() => stack.length > 0),
    compound_depth: vi.fn(() => stack.length),
    compound_begin: vi.fn(() => {
      stack.push(next)
      return next++
    }),
    compound_end: vi.fn((k: bigint) => {
      if (stack.at(-1) !== k) throw new Error('CompoundTokenMismatch')
      stack.pop()
      return '{}'
    }),
    compound_abort: vi.fn((k: bigint) => {
      if (stack.at(-1) !== k) throw new Error('CompoundTokenMismatch')
      stack.pop()
    }),
  }
  return s
}

/** An owner that answers its forward and aborts its compound when unwound. */
function owned(scene: Scene, forward: number, log: string[]): OpenCompound {
  const entry: OpenCompound = beginCompound(scene, 1, forward, () => {
    log.push(`unwound ${forward}`)
    abortCompound(scene, entry)
  })
  return entry
}

describe('the compounds this window holds', () => {
  it('ends the ones above, innermost first, before ending an outer one', () => {
    const k = kernel()
    const scene = k as unknown as Scene
    const log: string[] = []
    const outer = owned(scene, 10, log)
    owned(scene, 20, log)
    owned(scene, 30, log)
    expect(innermostForward(scene)).toBe(30)
    expect(isForwardOpen(scene, 20)).toBe(true)
    endCompound(scene, outer)
    expect(log).toEqual(['unwound 30', 'unwound 20'])
    expect(k.compound_abort.mock.calls.map(([t]) => t)).toEqual([3n, 2n])
    expect(k.compound_end).toHaveBeenCalledWith(1n)
    expect(k.stack).toEqual([])
    expect(openCompoundCount(scene)).toBe(0)
    expect(isForwardOpen(scene, 20)).toBe(false)
  })

  it('aborts a nested one whose owner leaves it open', () => {
    const k = kernel()
    const scene = k as unknown as Scene
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    const outer = owned(scene, 10, [])
    beginCompound(scene, 1, 20, () => {})
    abortCompound(scene, outer)
    expect(k.stack).toEqual([])
    expect(openCompoundCount(scene)).toBe(0)
    err.mockRestore()
  })

  it('never closes a compound it did not open, and aborts its own stranded one once that is the innermost', () => {
    const k = kernel()
    const scene = k as unknown as Scene
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    const mine = owned(scene, 10, [])
    // Someone else (the test harness) opens one on top.
    k.stack.push(99n)
    abortCompound(scene, mine)
    // The kernel refused: the other compound is innermost. Nothing changed.
    expect(k.stack).toEqual([1n, 99n])
    expect(openCompoundCount(scene)).toBe(0)
    // The other one ends; the next close of anything of this window's sweeps.
    k.stack.pop()
    const next = owned(scene, 20, [])
    endCompound(scene, next)
    expect(k.stack).toEqual([])
    expect(err).toHaveBeenCalledWith(expect.stringContaining('was aborted'))
    err.mockRestore()
  })

  it('releases a stranded compound when the busy state next changes, and reports it once', () => {
    const k = kernel()
    const scene = k as unknown as Scene
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    const mine = owned(scene, 10, [])
    k.stack.push(99n)
    setBusy({ plugin: 'someone.else', label: null, startedAt: 0, cancel: () => {} })
    abortCompound(scene, mine)
    // Other compounds of this window come and go above the foreign one.
    for (let i = 0; i < 3; i++) endCompound(scene, owned(scene, 20 + i, []))
    const stillHolds = err.mock.calls.filter(([m]) => String(m).includes('still holds'))
    expect(stillHolds).toHaveLength(1)
    expect(k.stack).toEqual([1n, 99n])
    // The foreign one ends; nothing of this window's closes after it.
    k.stack.pop()
    setBusy(null)
    expect(k.stack).toEqual([])
    err.mockRestore()
  })

  it('forgets an entry the kernel closed while refusing to end it', () => {
    const k = kernel()
    const scene = k as unknown as Scene
    const entry = owned(scene, 10, [])
    k.compound_end.mockImplementationOnce(() => {
      k.stack.pop()
      throw new Error('TransactionHistoryDisturbed')
    })
    expect(() => endCompound(scene, entry)).toThrow('TransactionHistoryDisturbed')
    expect(openCompoundCount(scene)).toBe(0)
    abortCompound(scene, entry)
    expect(k.compound_abort).not.toHaveBeenCalled()
  })
})
