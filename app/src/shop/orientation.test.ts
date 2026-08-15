// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { computeOrientation, useShopOrientation } from './orientation'

/** Sets `window.innerWidth`/`innerHeight` (jsdom's own are read-only own
 *  properties, not plain assignable fields) and fires `resize` — the same
 *  live-dimension-change shape a real rotation produces. */
function stubWindowSize(widthPx: number, heightPx: number): void {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: widthPx })
  Object.defineProperty(window, 'innerHeight', { configurable: true, value: heightPx })
}

const ORIGINAL_WIDTH = window.innerWidth
const ORIGINAL_HEIGHT = window.innerHeight

afterEach(() => {
  stubWindowSize(ORIGINAL_WIDTH, ORIGINAL_HEIGHT)
})

describe('computeOrientation', () => {
  it('reads a wider-than-tall viewport (844x390) as landscape', () => {
    expect(computeOrientation(844, 390)).toBe('landscape')
  })

  it('reads a taller-than-wide viewport (390x844) as portrait', () => {
    expect(computeOrientation(390, 844)).toBe('portrait')
  })

  it('reads an exactly square viewport as portrait (no square layout exists)', () => {
    expect(computeOrientation(500, 500)).toBe('portrait')
  })
})

describe('useShopOrientation', () => {
  it('reflects the window size at mount', () => {
    stubWindowSize(390, 844)
    const { result } = renderHook(() => useShopOrientation())
    expect(result.current).toBe('portrait')
  })

  it('re-evaluates on a resize event (a rotation)', () => {
    stubWindowSize(390, 844)
    const { result } = renderHook(() => useShopOrientation())
    expect(result.current).toBe('portrait')

    act(() => {
      stubWindowSize(844, 390)
      window.dispatchEvent(new Event('resize'))
    })
    expect(result.current).toBe('landscape')
  })

  it('re-evaluates on an orientationchange event', () => {
    stubWindowSize(844, 390)
    const { result } = renderHook(() => useShopOrientation())
    expect(result.current).toBe('landscape')

    act(() => {
      stubWindowSize(390, 844)
      window.dispatchEvent(new Event('orientationchange'))
    })
    expect(result.current).toBe('portrait')
  })

  it('stops listening after unmount', () => {
    stubWindowSize(390, 844)
    const { result, unmount } = renderHook(() => useShopOrientation())
    unmount()
    act(() => {
      stubWindowSize(844, 390)
      window.dispatchEvent(new Event('resize'))
    })
    // No re-render happened post-unmount to observe — this mainly proves
    // the listener teardown doesn't throw; `result.current` still holds
    // its last value from before unmount.
    expect(result.current).toBe('portrait')
  })
})
