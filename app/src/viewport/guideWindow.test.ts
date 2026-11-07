import { describe, it, expect } from 'vitest'
import {
  windowFor, guideReach, guideSpanOf, clampToSpan, stretchToUse, guideOverlayPositions,
  GUIDE_OVERLAY_HALF_LENGTH, GUIDE_POINT_MARKER_HALF_SIZE, type GuideWindow,
} from './guideWindow'

describe('windowFor', () => {
  it('centres on the focus and snaps the half-length up to a power of two', () => {
    const w = windowFor(0.3, 0.07, null)
    expect(w.center).toBe(0.3)
    expect(w.half).toBe(0.125)  // 0.07 × 1.5 headroom, rounded up to 2^-3
  })

  it('never draws less than a millimetre of half-length', () => {
    expect(windowFor(0, 0, null).half).toBeGreaterThanOrEqual(1e-3)
  })

  it('keeps the same window object while the focus stays near its centre', () => {
    const first = windowFor(1, 0.5, null)
    expect(windowFor(1.05, 0.5, first)).toBe(first)
    expect(windowFor(1 + (first.half - 0.5), 0.5, first)).toBe(first)
  })

  it('keeps the window through a small dolly in either direction', () => {
    const first = windowFor(0, 0.5, null)
    expect(first.half).toBe(1)
    expect(windowFor(0, 0.45, first)).toBe(first)
    expect(windowFor(0, 0.95, first)).toBe(first)
  })

  it('recentres once the view would reach past the end of the window', () => {
    const first = windowFor(0, 0.5, null)
    const drift = first.half - 0.5 + 0.01
    const next = windowFor(drift, 0.5, first)
    expect(next).not.toBe(first)
    expect(next.center).toBeCloseTo(drift, 12)
    expect(next.half).toBe(first.half)
  })

  it('grows when the view needs more than the window covers', () => {
    const first = windowFor(0, 0.5, null)
    const next = windowFor(0, first.half * 1.5, first)
    expect(next.half).toBeGreaterThanOrEqual(first.half * 1.5)
    expect(next.half).toBeGreaterThan(first.half)
  })

  it('shrinks after a big zoom-in rather than keep a huge stale window', () => {
    const wide: GuideWindow = { center: 0, half: 64 }
    const next = windowFor(0, 0.07, wide)
    expect(next.half).toBe(0.125)
  })

  it('always covers what the view needs on both sides of the focus', () => {
    let w: GuideWindow | null = null
    for (let i = 0; i < 400; i++) {
      const focus = Math.sin(i / 17) * 3
      const needed = 0.05 + (i % 40) * 0.07
      w = windowFor(focus, needed, w)
      expect(w.center - w.half).toBeLessThanOrEqual(focus - needed + 1e-9)
      expect(w.center + w.half).toBeGreaterThanOrEqual(focus + needed - 1e-9)
    }
  })
})

describe('guideReach', () => {
  const view = (perspective: boolean) => ({ eye: [0, -3, 0] as const, dir: [0, 1, 0] as const, focusDepth: 3, perspective })

  it('is the base reach for a guide square-on to the view', () => {
    expect(guideReach(1, [1, 0, 0], [0, 0, 0], view(false))).toBeCloseTo(1, 12)
  })

  it('stretches by 1/sin for a guide at a shallow angle to the view direction', () => {
    const a = (30 * Math.PI) / 180
    expect(guideReach(1, [Math.sin(a), Math.cos(a), 0], [0, 0, 0], view(false))).toBeCloseTo(2, 9)
  })

  it('caps a guide pointing straight along the view at 10×', () => {
    expect(guideReach(1, [0, 1, 0], [0, 0, 0], view(false))).toBeCloseTo(10, 12)
  })

  it('in perspective, scales by how much deeper the guide is than the focus', () => {
    // Nearest point 6 m beyond the focus: depth 9 vs focus depth 3.
    expect(guideReach(1, [1, 0, 0], [0, 6, 0], view(true))).toBeCloseTo(3, 12)
    expect(guideReach(1, [1, 0, 0], [0, 6, 0], view(false))).toBeCloseTo(1, 12)
  })

  it('never shrinks a guide nearer than the focus, and caps the depth scale', () => {
    expect(guideReach(1, [1, 0, 0], [0, -2, 0], view(true))).toBeCloseTo(1, 12)
    expect(guideReach(1, [1, 0, 0], [0, 1000, 0], view(true))).toBeCloseTo(20, 12)
  })
})

describe('guideSpanOf / clampToSpan', () => {
  it('treats a 6-element line as infinite and reads from/to from 8', () => {
    expect(guideSpanOf([0, 0, 0, 1, 0, 0])).toEqual({ lo: -Infinity, hi: Infinity })
    expect(guideSpanOf([0, 0, 0, 1, 0, 0, 0, 0.5])).toEqual({ lo: 0, hi: 0.5 })
    expect(guideSpanOf([0, 0, 0, 1, 0, 0, 2, 1])).toEqual({ lo: 1, hi: 2 })
  })

  it('clamps a window to the span and returns null when it misses', () => {
    expect(clampToSpan(-5, 5, 0, Infinity)).toEqual([0, 5])
    expect(clampToSpan(-5, 5, 0, 0.0127)).toEqual([0, 0.0127])
    expect(clampToSpan(-9, -6, 0, Infinity)).toBeNull()
    expect(clampToSpan(7, 9, 0, 1)).toBeNull()
  })

  it('uses a segment whole wherever it lies, and clamps a ray', () => {
    expect(stretchToUse(-50, 50, 120, 130)).toEqual([120, 130])
    expect(stretchToUse(-50, 50, 0, Infinity)).toEqual([0, 50])
    expect(stretchToUse(-50, -10, 0, Infinity)).toBeNull()
    expect(stretchToUse(-50, 50, -Infinity, Infinity)).toEqual([-50, 50])
  })
})

describe('guideOverlayPositions', () => {
  it('draws an infinite line the overlay half-length either side of its origin, along its unit direction', () => {
    const h = GUIDE_OVERLAY_HALF_LENGTH
    expect(guideOverlayPositions('line', [1, 2, 3, 0, 2, 0])).toEqual([1, 2 - h, 3, 1, 2 + h, 3])
  })

  it('draws a segment guide whole, wherever it lies along its line', () => {
    expect(guideOverlayPositions('line', [0, 0, 0, 1, 0, 0, 100, 102])).toEqual([100, 0, 0, 102, 0, 0])
  })

  it('draws a point guide as a three-axis cross', () => {
    const k = GUIDE_POINT_MARKER_HALF_SIZE
    expect(guideOverlayPositions('point', [1, 1, 1])).toEqual([
      1 - k, 1, 1, 1 + k, 1, 1,
      1, 1 - k, 1, 1, 1 + k, 1,
      1, 1, 1 - k, 1, 1, 1 + k,
    ])
  })

  it('draws nothing for a degenerate line, a missed span, or an unknown kind', () => {
    expect(guideOverlayPositions('line', [0, 0, 0, 0, 0, 0])).toBeNull()
    expect(guideOverlayPositions('line', [0, 0, 0, 1, 0, 0, 1000, NaN])).toBeNull()
    expect(guideOverlayPositions('arc', [0, 0, 0])).toBeNull()
  })
})
