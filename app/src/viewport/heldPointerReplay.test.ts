import { describe, expect, it } from 'vitest'
import { followsViewPlane, leftCanvasBounds, replaysWithoutSnap, shouldReplayHeldPointer, type HeldPointerState } from './heldPointerReplay'

describe('shouldReplayHeldPointer', () => {
  const canvas = { id: 'canvas' } as unknown as Element
  const palette = { id: 'palette' } as unknown as Element
  const container = { contains: (n: Node | null) => n === (canvas as unknown as Node) }
  const base: HeldPointerState = {
    hasHeldRay: true,
    snapPathLive: true,
    pointerInside: true,
    elementAtPoint: canvas,
    container,
  }

  it('replays with the pointer resting on the canvas', () => {
    expect(shouldReplayHeldPointer(base)).toBe(true)
  })

  it('does not replay once the pointer has left the canvas, though the point it left from is still canvas', () => {
    expect(shouldReplayHeldPointer({ ...base, pointerInside: false })).toBe(false)
  })

  it('does not replay when an overlay covers the point the pointer rests on', () => {
    expect(shouldReplayHeldPointer({ ...base, elementAtPoint: palette })).toBe(false)
    expect(shouldReplayHeldPointer({ ...base, elementAtPoint: null })).toBe(false)
  })

  it('does not replay without a held ray or while another gesture owns the pointer', () => {
    expect(shouldReplayHeldPointer({ ...base, hasHeldRay: false })).toBe(false)
    expect(shouldReplayHeldPointer({ ...base, snapPathLive: false })).toBe(false)
  })
})

describe('replaysWithoutSnap', () => {
  it('a screen-space drag tool replays with a null snap; any other tool resolves one', () => {
    expect(replaysWithoutSnap({ onPointerRawMove: () => {} })).toBe(true)
    expect(replaysWithoutSnap({ onPointerMove: () => {} })).toBe(false)
  })
})

describe('leftCanvasBounds', () => {
  const rect = { left: 100, top: 50, right: 900, bottom: 650 }
  it('a leave at a point outside the canvas is a real departure', () => {
    expect(leftCanvasBounds(50, 300, rect)).toBe(true) // the left rail
    expect(leftCanvasBounds(500, 20, rect)).toBe(true) // the menu bar
    expect(leftCanvasBounds(900, 300, rect)).toBe(true) // the right edge itself
  })
  it('a leave at a point inside the canvas is an overlay coming up over a resting pointer', () => {
    expect(leftCanvasBounds(500, 300, rect)).toBe(false)
  })
})

describe('followsViewPlane', () => {
  it('only a tool that opts in gets the idle-key, Shift and camera-settle hover replay', () => {
    expect(followsViewPlane({ followsViewPlane: true })).toBe(true)
    expect(followsViewPlane({ followsViewPlane: false })).toBe(false)
    expect(followsViewPlane({})).toBe(false)
    // Only a literal true opts in, not any truthy value.
    expect(followsViewPlane({ followsViewPlane: 'yes' })).toBe(false)
  })
})
