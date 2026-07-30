import { describe, it, expect } from 'vitest'
import { MultiClickTracker, MULTI_CLICK_MS, MULTI_CLICK_SLOP_PX } from './multiClick'

/** A press at (x, y) at time t on the primary mouse button. */
function press(t: number, x = 100, y = 100, button = 0, pointerType = 'mouse') {
  return { timeStamp: t, clientX: x, clientY: y, button, pointerType }
}

describe('MultiClickTracker', () => {
  it('counts a lone press as 1', () => {
    const tracker = new MultiClickTracker()
    expect(tracker.press(press(0))).toBe(1)
  })

  it('counts the second press of a fast double-click as 2 — the case Chromium reports as detail=0', () => {
    // This is the whole point of the class: on Chromium `pointerdown.detail`
    // is 0 for BOTH presses, so without this the phantom second press is
    // routed to the tool as an ordinary click.
    const tracker = new MultiClickTracker()
    expect(tracker.press(press(0))).toBe(1)
    expect(tracker.press(press(80))).toBe(2)
  })

  it('keeps counting past 2, like `detail` does', () => {
    const tracker = new MultiClickTracker()
    expect(tracker.press(press(0))).toBe(1)
    expect(tracker.press(press(80))).toBe(2)
    expect(tracker.press(press(160))).toBe(3)
  })

  it('restarts at 1 once the presses are too far apart in time', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0))
    expect(tracker.press(press(MULTI_CLICK_MS + 1))).toBe(1)
  })

  it('treats a press exactly at the time limit as still paired (inclusive bound)', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0))
    expect(tracker.press(press(MULTI_CLICK_MS))).toBe(2)
  })

  it('restarts at 1 once the presses are too far apart in space', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100))
    expect(tracker.press(press(50, 100 + MULTI_CLICK_SLOP_PX + 1, 100))).toBe(1)
  })

  it('measures the slop as a RADIUS, not per-axis', () => {
    // A diagonal move of (slop, slop) has length slop*sqrt(2) — outside the
    // circle even though each axis alone is within it. Testing per-axis would
    // accept a press up to 41% further away than intended.
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100))
    expect(tracker.press(press(50, 100 + MULTI_CLICK_SLOP_PX, 100 + MULTI_CLICK_SLOP_PX))).toBe(1)
  })

  it('pairs a press exactly on the slop radius (inclusive bound)', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100))
    expect(tracker.press(press(50, 100 + MULTI_CLICK_SLOP_PX, 100))).toBe(2)
  })

  it('does not pair presses from different buttons', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100, 0))
    expect(tracker.press(press(50, 100, 100, 2))).toBe(1)
  })

  it('does not pair presses from different pointer types', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100, 0, 'mouse'))
    expect(tracker.press(press(50, 100, 100, 0, 'pen'))).toBe(1)
  })

  it('starts a fresh sequence when timestamps go backwards', () => {
    // A negative gap is not evidence of a fast second click; without the
    // `dt >= 0` guard it would satisfy `dt <= MULTI_CLICK_MS` and pair up.
    const tracker = new MultiClickTracker()
    tracker.press(press(1000))
    expect(tracker.press(press(10))).toBe(1)
  })

  it('reset() breaks the sequence even for presses that would otherwise pair', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0))
    tracker.reset()
    expect(tracker.press(press(80))).toBe(1)
  })

  it('a first press long after construction still counts as 1, not as a continuation', () => {
    // The initial `lastTime` must not read as "just happened" — a sentinel of
    // 0 would make the very first press at t=50 look like a second click.
    const tracker = new MultiClickTracker()
    expect(tracker.press(press(50))).toBe(1)
  })
})
