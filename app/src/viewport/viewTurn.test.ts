import { describe, it, expect } from 'vitest'
import { viewTurnDirection, viewTurnEye, viewTurnRotation, type V3 } from './viewTurn'

const POLE_TILT = 0.001
const n = (v: V3): V3 => {
  const l = Math.hypot(...v)
  return [v[0] / l, v[1] / l, v[2] / l]
}
const FRONT: V3 = [0, -1, 0]
const BACK: V3 = [0, 1, 0]
const RIGHT: V3 = [1, 0, 0]
const TOP: V3 = n([0, -POLE_TILT, 1])
const BOTTOM: V3 = n([0, -POLE_TILT, -1])
const ISO: V3 = n([1, -1, 1])
const ISO_BELOW_OPPOSITE: V3 = n([-1, 1, -1])

const TARGET: V3 = [2, -3, 0.5]
const DIST = 9
const STEPS = 200
const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
const close = (a: V3, b: V3, digits = 9): void => a.forEach((c, i) => expect(c).toBeCloseTo(b[i]!, digits))

/** Every frame of a turn: the eye stays on the sphere about the target,
 *  starts and ends where asked, and the target never moves. */
function expectOrbitTurn(from: V3, to: V3): void {
  const rot = viewTurnRotation(from, to)
  const target: V3 = [...TARGET]
  let minDist = Infinity
  let maxDist = 0
  for (let i = 0; i <= STEPS; i++) {
    const eye = viewTurnEye(target, DIST, from, rot, i / STEPS)
    const d = Math.hypot(...sub(eye, target))
    minDist = Math.min(minDist, d)
    maxDist = Math.max(maxDist, d)
  }
  expect(minDist).toBeCloseTo(DIST, 9)
  expect(maxDist).toBeCloseTo(DIST, 9)
  expect(target).toEqual(TARGET)
  close(viewTurnDirection(from, rot, 0), from)
  close(viewTurnDirection(from, rot, 1), to)
}

describe('viewTurn — a die turn orbits the fixed target at a fixed distance', () => {
  it('90° turns: Front to Right, Front to Top, Iso to Front', () => {
    expectOrbitTurn(FRONT, RIGHT)
    expectOrbitTurn(FRONT, TOP)
    expectOrbitTurn(ISO, FRONT)
    expect(viewTurnRotation(FRONT, RIGHT).angle).toBeCloseTo(Math.PI / 2, 12)
  })

  it('180° flips: Front to Back, Top to Bottom, a corner to its opposite', () => {
    expectOrbitTurn(FRONT, BACK)
    expectOrbitTurn(BACK, FRONT)
    expectOrbitTurn(TOP, BOTTOM)
    expectOrbitTurn(BOTTOM, TOP)
    expectOrbitTurn(ISO, ISO_BELOW_OPPOSITE)
  })

  it('a horizontal flip swings round blue: it stays level the whole way (never over the top or through the target)', () => {
    const rot = viewTurnRotation(FRONT, BACK)
    close(rot.axis.map(Math.abs) as V3, [0, 0, 1])
    for (let i = 0; i <= STEPS; i++) expect(viewTurnDirection(FRONT, rot, i / STEPS)[2]).toBeCloseTo(0, 12)
    // Halfway it is side-on, a full distance from the target.
    close(viewTurnDirection(FRONT, rot, 0.5).map(Math.abs) as V3, [1, 0, 0])
  })

  it('Top to Bottom swings over the front (the screen-right axis), not round the pole', () => {
    const rot = viewTurnRotation(TOP, BOTTOM)
    expect(Math.abs(rot.axis[0])).toBeCloseTo(1, 6)
    const mid = viewTurnDirection(TOP, rot, 0.5)
    expect(mid[1]).toBeLessThan(-0.99) // the Front side
    expect(Math.abs(mid[2])).toBeLessThan(0.01)
  })

  it('the axis choice is deterministic: the same turn takes the same path every time', () => {
    const a = viewTurnRotation(FRONT, BACK)
    const b = viewTurnRotation(FRONT, BACK)
    expect(a).toEqual(b)
  })

  it('a turn interrupted mid-flight by a second click restarts from where the eye is, still on the same sphere', () => {
    // Front → Back, cut at 40% by a click on Top: the second turn starts at
    // the interrupted direction, about the same target, at the same
    // distance, and lands exactly on Top.
    const first = viewTurnRotation(FRONT, BACK)
    const cut = viewTurnDirection(FRONT, first, 0.4)
    expect(Math.hypot(...sub(viewTurnEye(TARGET, DIST, FRONT, first, 0.4), TARGET))).toBeCloseTo(DIST, 9)
    expectOrbitTurn(cut, TOP)
  })

  it('a turn cancelled at any frame leaves the pivot where it was', () => {
    // The target is an input the turn never writes: whatever frame a camera
    // drag, a wheel or a fly cancels at, the orbit target is the true one.
    const rot = viewTurnRotation(TOP, BOTTOM)
    const target: V3 = [...TARGET]
    for (const t of [0.1, 0.5, 0.73]) {
      const eye = viewTurnEye(target, DIST, TOP, rot, t)
      expect(Math.hypot(...sub(eye, target))).toBeCloseTo(DIST, 9)
      expect(target).toEqual(TARGET)
    }
  })

  it('no turn at all when already there', () => {
    expect(viewTurnRotation(FRONT, FRONT).angle).toBe(0)
    close(viewTurnDirection(FRONT, viewTurnRotation(FRONT, FRONT), 0.5), FRONT)
  })
})
