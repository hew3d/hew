import { describe, it, expect } from 'vitest'
import { MultiClickTracker, MULTI_CLICK_MS, MULTI_CLICK_SLOP_PX, MULTI_CLICK_TOUCH_SLOP_PX } from './multiClick'

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
    tracker.release(press(5))
    expect(tracker.press(press(80))).toBe(2)
  })

  it('does NOT pair a press that follows a DRAG back to where the drag started', () => {
    // Measured against real Chromium: a press at P dragged to Q and released
    // there, followed within the window by a fresh press back at P, fires NO
    // `dblclick` — the browser pairs on where clicks are RELEASED, and these
    // released in different places. Suppressing that second press would drop
    // the gesture with nothing to replace it, since the tools that opt into
    // suppression arm inside `onPointerDown` and have no later fallback.
    //
    // Held at this level deliberately. Suppression applies only to tools
    // implementing `onDoubleClick`, and today both of those (Line, Push/Pull)
    // are click-move-click rather than press-drag-release, so no end-to-end
    // gesture currently reaches this shape — an e2e written against one of
    // them exercises an ordinary double-click instead, and passes whatever
    // the tracker does. The property still has to hold before a drag-based
    // tool grows an `onDoubleClick`, which is exactly what this pins.
    const tracker = new MultiClickTracker()
    expect(tracker.press(press(0, 100, 100))).toBe(1)
    tracker.release(press(60, 300, 300)) // travelled: a drag, not a click
    expect(tracker.press(press(120, 100, 100))).toBe(1)
  })

  it('a drag breaks a run that had already started', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0))
    tracker.release(press(5))
    expect(tracker.press(press(80))).toBe(2) // a genuine double-click so far
    tracker.release(press(140, 400, 400)) // ...whose second press became a drag
    expect(tracker.press(press(200))).toBe(1)
  })

  it('pairs against where the previous click was RELEASED, not where it was pressed', () => {
    // A click that shifts a pixel or two between press and release is still a
    // click; the browser compares the release points, so this does too.
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100))
    tracker.release(press(20, 103, 100)) // 3px: within slop, still a click
    // Now a press at the RELEASE point pairs...
    expect(tracker.press(press(80, 103, 100))).toBe(2)
  })

  it('an unreleased press anchors nothing — a second press cannot pair with it', () => {
    // Without a release the first press is still in flight; it is not yet a
    // click and must not be treated as one.
    const tracker = new MultiClickTracker()
    expect(tracker.press(press(0))).toBe(1)
    expect(tracker.press(press(80))).toBe(1)
  })

  it('keeps counting past 2, like `detail` does', () => {
    const tracker = new MultiClickTracker()
    expect(tracker.press(press(0))).toBe(1)
    tracker.release(press(5))
    expect(tracker.press(press(80))).toBe(2)
    tracker.release(press(85))
    expect(tracker.press(press(160))).toBe(3)
  })

  it('restarts at 1 once the presses are too far apart in time', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0))
    tracker.release(press(5))
    expect(tracker.press(press(MULTI_CLICK_MS + 6))).toBe(1)
  })

  it('treats a press exactly at the time limit as still paired (inclusive bound)', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0))
    tracker.release(press(0))
    expect(tracker.press(press(MULTI_CLICK_MS))).toBe(2)
  })

  it('restarts at 1 once the presses are too far apart in space', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100))
    tracker.release(press(5, 100, 100))
    expect(tracker.press(press(50, 100 + MULTI_CLICK_SLOP_PX + 1, 100))).toBe(1)
  })

  it('measures the slop as a RADIUS, not per-axis', () => {
    // A diagonal move of (slop, slop) has length slop*sqrt(2) — outside the
    // circle even though each axis alone is within it. Testing per-axis would
    // accept a press up to 41% further away than intended.
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100))
    tracker.release(press(5, 100, 100))
    expect(tracker.press(press(50, 100 + MULTI_CLICK_SLOP_PX, 100 + MULTI_CLICK_SLOP_PX))).toBe(1)
  })

  it('pairs a press exactly on the slop radius (inclusive bound)', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100))
    tracker.release(press(5, 100, 100))
    expect(tracker.press(press(50, 100 + MULTI_CLICK_SLOP_PX, 100))).toBe(2)
  })

  it('does not pair presses from different buttons', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100, 0))
    tracker.release(press(5, 100, 100, 0))
    expect(tracker.press(press(50, 100, 100, 2))).toBe(1)
  })

  it('does not pair presses from different pointer types', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0, 100, 100, 0, 'mouse'))
    tracker.release(press(5, 100, 100, 0, 'mouse'))
    expect(tracker.press(press(50, 100, 100, 0, 'pen'))).toBe(1)
  })

  it('starts a fresh sequence when timestamps go backwards', () => {
    // A negative gap is not evidence of a fast second click; without the
    // `dt >= 0` guard it would satisfy `dt <= MULTI_CLICK_MS` and pair up.
    const tracker = new MultiClickTracker()
    tracker.press(press(1000))
    tracker.release(press(1005))
    expect(tracker.press(press(10))).toBe(1)
  })

  it('reset() breaks the sequence even for presses that would otherwise pair', () => {
    const tracker = new MultiClickTracker()
    tracker.press(press(0))
    tracker.release(press(5))
    tracker.reset()
    expect(tracker.press(press(80))).toBe(1)
  })

  it('a first press long after construction still counts as 1, not as a continuation', () => {
    // The initial `lastTime` must not read as "just happened" — a sentinel of
    // 0 would make the very first press at t=50 look like a second click.
    const tracker = new MultiClickTracker()
    expect(tracker.press(press(50))).toBe(1)
  })

  // Shop-mode playtest finding 6: a real fingertip's second tap lands
  // nowhere near the mouse-tuned MULTI_CLICK_SLOP_PX (4px) even for a
  // deliberate double-tap on the same target — this is why the double-tap
  // zoom "worked in tests" (synthetic taps land pixel-identical) but not on
  // Kurt's real phone.
  describe('touch input uses a wider slop (MULTI_CLICK_TOUCH_SLOP_PX)', () => {
    it('pairs a touch double-tap that scattered well past the mouse slop but within the touch slop', () => {
      const tracker = new MultiClickTracker()
      tracker.press(press(0, 100, 100, 0, 'touch'))
      tracker.release(press(20, 100, 100, 0, 'touch'))
      // Comfortably past MULTI_CLICK_SLOP_PX (4) — would fail under the
      // mouse-tuned slop — but inside MULTI_CLICK_TOUCH_SLOP_PX (24).
      const scatter = MULTI_CLICK_SLOP_PX + 10
      expect(tracker.press(press(80, 100 + scatter, 100, 0, 'touch'))).toBe(2)
    })

    it('still rejects a touch second-tap past the touch slop itself', () => {
      const tracker = new MultiClickTracker()
      tracker.press(press(0, 100, 100, 0, 'touch'))
      tracker.release(press(20, 100, 100, 0, 'touch'))
      expect(tracker.press(press(80, 100 + MULTI_CLICK_TOUCH_SLOP_PX + 1, 100, 0, 'touch'))).toBe(1)
    })

    it('the touch slop also governs release\'s own drag-vs-click distinction', () => {
      const tracker = new MultiClickTracker()
      tracker.press(press(0, 100, 100, 0, 'touch'))
      // Travelled past the mouse slop but within the touch slop on RELEASE —
      // still a click (not a drag), so the run can still pair.
      const scatter = MULTI_CLICK_SLOP_PX + 10
      tracker.release(press(20, 100 + scatter, 100, 0, 'touch'))
      expect(tracker.press(press(80, 100 + scatter, 100, 0, 'touch'))).toBe(2)
    })

    it('a MOUSE press keeps the tight slop untouched even on the same tracker instance', () => {
      // A touch-capable desktop (rare, but real) shouldn't get touch's wider
      // slop for its own mouse input — the widening is keyed off THIS
      // press's own pointerType, not a sticky tracker-wide mode.
      const tracker = new MultiClickTracker()
      tracker.press(press(0, 100, 100, 0, 'mouse'))
      tracker.release(press(5, 100, 100, 0, 'mouse'))
      expect(tracker.press(press(50, 100 + MULTI_CLICK_SLOP_PX + 1, 100, 0, 'mouse'))).toBe(1)
    })
  })
})
