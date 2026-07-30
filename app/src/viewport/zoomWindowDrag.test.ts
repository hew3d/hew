import { describe, expect, it } from 'vitest'
import {
  beginZoomWindowDrag,
  updateZoomWindowDrag,
  finishZoomWindowDrag,
  ZOOM_WINDOW_DRAG_THRESHOLD_PX,
} from './zoomWindowDrag'

describe('zoomWindowDrag — arm/drag decision logic', () => {
  it('the very FIRST press-drag-release cycle arms and commits a rectangle (regression: "first drag ignored, second works")', () => {
    // A single, uninterrupted pointer sequence — no priming cycle, no prior
    // activation of any kind. If this needed two cycles to "wake up", the
    // rect below would be null.
    const armed = beginZoomWindowDrag(10, 10)
    const past = ZOOM_WINDOW_DRAG_THRESHOLD_PX + 20
    const { state: dragging, rect: liveRect } = updateZoomWindowDrag(armed, 10 + past, 10 + past, true)
    expect(dragging).not.toBeNull()
    expect(dragging?.active).toBe(true)
    expect(liveRect).toEqual({ minX: 10, minY: 10, maxX: 10 + past, maxY: 10 + past })

    const committed = finishZoomWindowDrag(dragging as NonNullable<typeof dragging>, 10 + past, 10 + past)
    expect(committed).toEqual({ minX: 10, minY: 10, maxX: 10 + past, maxY: 10 + past })
  })

  it('a second, independent activation behaves identically to the first (no hidden priming state)', () => {
    for (let i = 0; i < 2; i++) {
      const armed = beginZoomWindowDrag(0, 0)
      const { state: dragging, rect } = updateZoomWindowDrag(armed, 30, 30, true)
      expect(rect).not.toBeNull()
      const committed = finishZoomWindowDrag(dragging as NonNullable<typeof dragging>, 30, 30)
      expect(committed).toEqual({ minX: 0, minY: 0, maxX: 30, maxY: 30 })
    }
  })

  it('sub-threshold travel never arms — release is a plain click, no reframe rect', () => {
    const armed = beginZoomWindowDrag(100, 100)
    const tiny = ZOOM_WINDOW_DRAG_THRESHOLD_PX - 1
    const { state, rect } = updateZoomWindowDrag(armed, 100 + tiny, 100, true)
    expect(state?.active).toBe(false)
    expect(rect).toBeNull()
    const committed = finishZoomWindowDrag(state as NonNullable<typeof state>, 100 + tiny, 100)
    expect(committed).toBeNull()
  })

  it('travel exactly at the threshold arms on that same call', () => {
    const armed = beginZoomWindowDrag(0, 0)
    const { state, rect } = updateZoomWindowDrag(armed, ZOOM_WINDOW_DRAG_THRESHOLD_PX, 0, true)
    expect(state?.active).toBe(true)
    expect(rect).not.toBeNull()
  })

  it('once active, stays active even if the pointer drifts back under the threshold', () => {
    const armed = beginZoomWindowDrag(0, 0)
    const { state: active } = updateZoomWindowDrag(armed, 50, 0, true)
    expect(active?.active).toBe(true)
    const { state: stillActive, rect } = updateZoomWindowDrag(active as NonNullable<typeof active>, 1, 0, true)
    expect(stillActive?.active).toBe(true)
    expect(rect).toEqual({ minX: 0, minY: 0, maxX: 1, maxY: 0 })
  })

  it('a focus-loss release (buttons bit clear) aborts with no reframe', () => {
    const armed = beginZoomWindowDrag(0, 0)
    const { state: active } = updateZoomWindowDrag(armed, 50, 50, true)
    const { state: aborted, rect } = updateZoomWindowDrag(active as NonNullable<typeof active>, 60, 60, false)
    expect(aborted).toBeNull()
    expect(rect).toBeNull()
  })

  it('normalizes an inverted drag (release above/left of the press) into a min/max rect', () => {
    const armed = beginZoomWindowDrag(50, 50)
    const { state } = updateZoomWindowDrag(armed, 0, 0, true)
    const committed = finishZoomWindowDrag(state as NonNullable<typeof state>, 0, 0)
    expect(committed).toEqual({ minX: 0, minY: 0, maxX: 50, maxY: 50 })
  })
})
