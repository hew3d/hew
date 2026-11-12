import { describe, it, expect } from 'vitest'
import { PLANE_PIPS, cursorPlaneOf, pipClickAction, pipEnabledFor, pipWillReleaseFor, type PipContext } from './drawPlanePips'
import { axisDrawPlane, drawPlaneThrough, groundDrawPlane, type DrawPlane, type DrawPlaneState } from '../tools/drawPlane'
import { WORLD_DRAWING_AXES } from '../tools/drawingAxes'

const GREEN = axisDrawPlane(1, [0, 0, 0])
const r = (30 * Math.PI) / 180
const TURNED: DrawPlane = drawPlaneThrough([0, 0, 0], [Math.cos(r), Math.sin(r), 0])!

function ctx(over: Partial<PipContext> = {}): PipContext {
  const state: DrawPlaneState = { plane: GREEN, pip: 'green', hold: 'inferred', hover: null }
  return { state, pinnable: GREEN, capturing: false, frame: WORLD_DRAWING_AXES, readOnly: false, arrowReleases: false, customPlane: null, ...over }
}
const held = (pip: DrawPlaneState['pip'], hold: DrawPlaneState['hold'], plane: DrawPlane = GREEN): DrawPlaneState =>
  ({ plane, pip, hold, hover: null })

describe('pip clicks (view-die §3.3)', () => {
  it('idle: red, green and blue are the arrow keys → ← ↑ (lock, or release the one locked: the key handler decides)', () => {
    expect(pipClickAction('red', ctx())).toEqual({ kind: 'key', key: 'ArrowRight' })
    expect(pipClickAction('green', ctx())).toEqual({ kind: 'key', key: 'ArrowLeft' })
    expect(pipClickAction('blue', ctx())).toEqual({ kind: 'key', key: 'ArrowUp' })
    // The locked pip again is the same arrow again, which releases it.
    expect(pipClickAction('blue', ctx({ state: held('blue', 'locked', groundDrawPlane()) }))).toEqual({ kind: 'key', key: 'ArrowUp' })
  })

  it('idle under a Shift pin: another axis pip is its arrow (which displaces the pin); the pinned pip releases the pin', () => {
    const pinned = ctx({ state: held('green', 'pinned') })
    expect(pipClickAction('red', pinned)).toEqual({ kind: 'key', key: 'ArrowRight' })
    expect(pipClickAction('green', pinned)).toEqual({ kind: 'togglePin' })
  })

  it('during a gesture: an axis pip is the arrow key at that stage, even on the pinned pip', () => {
    const gesture = ctx({ state: held('green', 'pinned'), capturing: true, pinnable: null })
    expect(pipClickAction('green', gesture)).toEqual({ kind: 'key', key: 'ArrowLeft' })
    expect(pipClickAction('red', ctx({ state: held('green', 'anchored'), capturing: true, pinnable: null }))).toEqual({ kind: 'key', key: 'ArrowRight' })
  })

  it('custom: enabled only with a custom plane for it (pinned, tracked, or the live candidate); it pins or releases', () => {
    expect(pipEnabledFor('custom', ctx({ customPlane: TURNED }))).toBe(true)
    expect(pipClickAction('custom', ctx({ customPlane: TURNED }))).toEqual({ kind: 'pinCustom' })
    // Enabled while the filled pip is truthfully the ground the pointer
    // crossed (the candidate is the turned face).
    expect(pipEnabledFor('custom', ctx({ state: held('blue', 'inferred', groundDrawPlane()), pinnable: groundDrawPlane(), customPlane: TURNED }))).toBe(true)
    // A pinned custom plane: the click releases it.
    expect(pipClickAction('custom', ctx({ state: held('custom', 'pinned', TURNED), pinnable: TURNED, customPlane: TURNED }))).toEqual({ kind: 'pinCustom' })
    // An axis plane offered, or nothing: disabled.
    expect(pipEnabledFor('custom', ctx({ customPlane: GREEN }))).toBe(false)
    expect(pipEnabledFor('custom', ctx({ customPlane: null }))).toBe(false)
    expect(pipClickAction('custom', ctx({ customPlane: null }))).toEqual({ kind: 'none' })
  })

  it('custom during a gesture: nothing (there is nothing to pin mid-gesture)', () => {
    expect(pipClickAction('custom', ctx({ state: held('custom', 'anchored', TURNED), capturing: true, pinnable: null, customPlane: null }))).toEqual({ kind: 'none' })
  })

  it('custom follows moved drawing axes: a face along the moved frame\'s X is red, not custom', () => {
    const frame = { origin: [0, 0, 0] as [number, number, number], x: [Math.cos(r), Math.sin(r), 0] as [number, number, number], y: [-Math.sin(r), Math.cos(r), 0] as [number, number, number], z: [0, 0, 1] as [number, number, number] }
    expect(pipEnabledFor('custom', ctx({ customPlane: TURNED, frame }))).toBe(false)
  })

  it('a non-draw tool: every pip disabled, a click does nothing', () => {
    for (const pip of ['red', 'green', 'blue', 'custom'] as const) {
      expect(pipEnabledFor(pip, ctx({ state: null, pinnable: null }))).toBe(false)
      expect(pipClickAction(pip, ctx({ state: null, pinnable: null }))).toEqual({ kind: 'none' })
    }
  })

  it('read-only (Shop Mode): nothing', () => {
    expect(pipClickAction('red', ctx({ readOnly: true }))).toEqual({ kind: 'none' })
  })
})

describe('pipWillRelease — the "Release" tooltip only when a click releases (view-die §3.3)', () => {
  it('the locked pip while its arrow releases the lock', () => {
    const locked = held('blue', 'locked', groundDrawPlane())
    expect(pipWillReleaseFor('blue', ctx({ state: locked, arrowReleases: true }))).toBe(true)
    expect(pipWillReleaseFor('red', ctx({ state: locked, arrowReleases: false }))).toBe(false)
  })

  it('Line mid-chain: the lit, padlocked pip does not release (its arrow locks the segment axis)', () => {
    const chain = ctx({ state: held('red', 'locked', axisDrawPlane(0, [0, 0, 0])), capturing: true, pinnable: null, arrowReleases: false })
    expect(pipWillReleaseFor('red', chain)).toBe(false)
    expect(pipClickAction('red', chain)).toEqual({ kind: 'key', key: 'ArrowRight' })
  })

  it('the pinned pip of a Shift pin, and the custom pip of a pinned custom plane', () => {
    expect(pipWillReleaseFor('green', ctx({ state: held('green', 'pinned') }))).toBe(true)
    expect(pipWillReleaseFor('red', ctx({ state: held('green', 'pinned') }))).toBe(false)
    expect(pipWillReleaseFor('custom', ctx({ state: held('custom', 'pinned', TURNED), pinnable: TURNED, customPlane: TURNED }))).toBe(true)
    // A custom plane only tracked, not pinned: the click pins.
    expect(pipWillReleaseFor('custom', ctx({ pinnable: TURNED, customPlane: TURNED }))).toBe(false)
  })

  it('never for a disabled pip, and never while merely inferred', () => {
    for (const pip of PLANE_PIPS) {
      expect(pipWillReleaseFor(pip, ctx({ state: null, pinnable: null, arrowReleases: true }))).toBe(false)
      expect(pipWillReleaseFor(pip, ctx())).toBe(false)
    }
  })
})

describe('cursorPlaneOf — the ring and chip word name only a plane the click acts on', () => {
  it('names the plane and its hold while hovering', () => {
    expect(cursorPlaneOf({ plane: GREEN, pip: 'green', hold: 'locked', hover: { kind: 'space', point: [0, 0, 0] } }))
      .toEqual({ pip: 'green', hold: 'locked' })
  })

  it('nothing for no draw tool, or no hover (off the canvas, or an entered object off its faces)', () => {
    expect(cursorPlaneOf(null)).toBeUndefined()
    expect(cursorPlaneOf({ plane: GREEN, pip: 'green', hold: 'inferred', hover: null })).toBeUndefined()
  })
})
