import { describe, expect, it, vi } from 'vitest'
import { noteDroppedFor } from './openPathInLibrary'

describe('noteDroppedFor: File > Open drops the remote note only when the shell says the path is a Library item', () => {
  it('follows the shell: a name means inside', async () => {
    const ask = vi.fn(async () => 'Components/Leg.hew')
    expect(await noteDroppedFor('/lib-link/Components/Leg.hew', ask)).toBe(true)
    expect(ask).toHaveBeenCalledWith('/lib-link/Components/Leg.hew')
  })

  it('follows the shell: null means outside', async () => {
    expect(await noteDroppedFor('/lib/../elsewhere/a.hew', async () => null)).toBe(false)
  })

  it('does not second-guess the shell with its own string compare', async () => {
    // A string compare would call this inside; the shell says it is not.
    expect(await noteDroppedFor('/home/k/Library/Components/chair.hew', async () => null)).toBe(false)
    // A string compare would call this outside; the shell says it is.
    expect(await noteDroppedFor('/somewhere/else/chair.hew', async () => 'chair.hew')).toBe(true)
  })

  it('an invoke error counts as not a Library item (bytes stay unchanged)', async () => {
    const ask = vi.fn(async () => {
      throw new Error('invoke failed')
    })
    expect(await noteDroppedFor('/lib/a.hew', ask)).toBe(false)
  })
})
