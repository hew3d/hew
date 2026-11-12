import { describe, it, expect, vi } from 'vitest'
import { renderHook } from '@testing-library/react'
import { useClearSelectionOnDrawTool } from './useClearSelectionOnDrawTool'

const DRAW = new Set(['Line', 'Rectangle'])

describe('useClearSelectionOnDrawTool', () => {
  it('clears once when a draw tool is chosen at top level', () => {
    const clear = vi.fn()
    const { rerender } = renderHook((p: { tool: string }) => useClearSelectionOnDrawTool(p.tool, DRAW, false, clear), {
      initialProps: { tool: 'Select' },
    })
    expect(clear).not.toHaveBeenCalled()
    rerender({ tool: 'Line' })
    expect(clear).toHaveBeenCalledTimes(1)
  })

  it('does not clear again on a rerender that only changes the clear callback (a document change)', () => {
    const { rerender } = renderHook(
      (p: { clear: () => void }) => useClearSelectionOnDrawTool('Line', DRAW, false, p.clear),
      { initialProps: { clear: vi.fn() } },
    )
    const later = vi.fn()
    rerender({ clear: later })
    rerender({ clear: later })
    expect(later).not.toHaveBeenCalled()
  })

  it('keeps an in-context selection', () => {
    const clear = vi.fn()
    renderHook(() => useClearSelectionOnDrawTool('Line', DRAW, true, clear))
    expect(clear).not.toHaveBeenCalled()
  })
})
