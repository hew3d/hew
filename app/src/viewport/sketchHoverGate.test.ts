import { describe, it, expect } from 'vitest'
import { SketchHoverGate, HOVER_POLL_THROTTLE_MS } from './sketchHoverGate'

describe('SketchHoverGate.shouldPoll', () => {
  it('allows the first poll immediately', () => {
    const gate = new SketchHoverGate()
    expect(gate.shouldPoll(0)).toBe(true)
  })

  it('throttles polls closer together than HOVER_POLL_THROTTLE_MS', () => {
    const gate = new SketchHoverGate()
    expect(gate.shouldPoll(1000)).toBe(true)
    expect(gate.shouldPoll(1000 + HOVER_POLL_THROTTLE_MS - 1)).toBe(false)
  })

  it('allows a new poll once the throttle window has elapsed', () => {
    const gate = new SketchHoverGate()
    expect(gate.shouldPoll(1000)).toBe(true)
    expect(gate.shouldPoll(1000 + HOVER_POLL_THROTTLE_MS)).toBe(true)
  })

  it('does not advance the clock on a throttled (false) call', () => {
    const gate = new SketchHoverGate()
    expect(gate.shouldPoll(1000)).toBe(true)
    expect(gate.shouldPoll(1050)).toBe(false) // still inside the window
    // Had the clock advanced on the throttled call, this would also be
    // inside a *new* 100ms window measured from 1050 and stay false.
    expect(gate.shouldPoll(1100)).toBe(true)
  })
})

describe('SketchHoverGate.update (edge-detect)', () => {
  it('emits true the first time hovering becomes true', () => {
    const gate = new SketchHoverGate()
    expect(gate.update(true)).toBe(true)
  })

  it('emits nothing (null) on repeated identical polls', () => {
    const gate = new SketchHoverGate()
    expect(gate.update(true)).toBe(true)
    expect(gate.update(true)).toBeNull()
    expect(gate.update(true)).toBeNull()
  })

  it('emits false on the transition back to not-hovering', () => {
    const gate = new SketchHoverGate()
    gate.update(true)
    expect(gate.update(false)).toBe(false)
  })

  it('emits nothing for repeated false polls when already false (initial state)', () => {
    const gate = new SketchHoverGate()
    expect(gate.update(false)).toBeNull()
  })
})

describe('SketchHoverGate.pause', () => {
  it('emits a false transition when paused while hovering was true', () => {
    const gate = new SketchHoverGate()
    gate.update(true)
    expect(gate.pause()).toBe(false)
  })

  it('emits nothing when paused while already not-hovering', () => {
    const gate = new SketchHoverGate()
    expect(gate.pause()).toBeNull()
  })

  it('resets the throttle clock so the next poll after a pause is not held back', () => {
    const gate = new SketchHoverGate()
    expect(gate.shouldPoll(1000)).toBe(true)
    gate.pause()
    // Without the reset this would still be inside the 1000+100ms window.
    expect(gate.shouldPoll(1010)).toBe(true)
  })

  it('a repeated pause after the first keeps emitting null (no repeated false spam)', () => {
    const gate = new SketchHoverGate()
    gate.update(true)
    expect(gate.pause()).toBe(false)
    expect(gate.pause()).toBeNull()
  })
})

describe('SketchHoverGate.reset', () => {
  it('resets the throttle clock so the next shouldPoll is not held back', () => {
    const gate = new SketchHoverGate()
    expect(gate.shouldPoll(1000)).toBe(true)
    gate.reset()
    // Without the reset this would still be inside the 1000+100ms window.
    expect(gate.shouldPoll(1010)).toBe(true)
  })

  it('does NOT touch the last-emitted state (unlike pause)', () => {
    const gate = new SketchHoverGate()
    expect(gate.update(true)).toBe(true)
    gate.reset()
    // A repeated `update(true)` after a bare reset still reads as unchanged —
    // reset() must not force a false emission the way pause() does.
    expect(gate.update(true)).toBeNull()
  })

  it('leaves a genuine transition after reset free to emit normally', () => {
    const gate = new SketchHoverGate()
    gate.update(true)
    gate.reset()
    // The document changed and the cursor is now over empty ground — the
    // caller's manual re-poll feeds `false` in, and it should still emit the
    // real transition.
    expect(gate.update(false)).toBe(false)
  })
})
