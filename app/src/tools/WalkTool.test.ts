import { describe, it, expect, vi } from 'vitest'
import { WalkTool, WALK_MOVE_M_PER_PIXEL, WALK_TURN_RAD_PER_PIXEL } from './WalkTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { V3 } from './cameraWalkMath'
import { DEFAULT_EYE_HEIGHT_M, headingFromForward } from './cameraWalkMath'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
function makeSnap(x: number, y: number, z: number): Snap {
  return { x, y, z, kind: 'ground' }
}

function makeTool(overrides: { eye?: V3; forward?: V3; eyeHeight?: number } = {}) {
  const eye = overrides.eye ?? ([0, 0, DEFAULT_EYE_HEIGHT_M] as V3)
  const forward = overrides.forward ?? ([1, 0, 0] as V3)
  let eyeHeight = overrides.eyeHeight ?? DEFAULT_EYE_HEIGHT_M
  const applyPose = vi.fn()
  const onEscapeToSelect = vi.fn()
  const onMeasurement = vi.fn()
  const setEyeHeight = vi.fn((h: number) => {
    eyeHeight = h
  })
  const tool = new WalkTool(
    () => eye,
    () => forward,
    () => eyeHeight,
    setEyeHeight,
    applyPose,
    onEscapeToSelect,
    onMeasurement,
  )
  return { tool, applyPose, onEscapeToSelect, onMeasurement, setEyeHeight }
}

describe('WalkTool', () => {
  it('dragging up walks forward along the horizontal forward direction', () => {
    const { tool, applyPose } = makeTool({ forward: [1, 0, 0] })
    tool.onPointerRawDown(100, 100)
    tool.onPointerRawMove(100, 0, 1, { shift: false }) // drag up 100px -> forward
    const [eye] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(eye[0]).toBeCloseTo(100 * WALK_MOVE_M_PER_PIXEL, 6)
    expect(eye[1]).toBeCloseTo(0, 9)
    expect(eye[2]).toBeCloseTo(DEFAULT_EYE_HEIGHT_M, 9) // height unchanged without Shift
  })

  it('dragging down walks backward', () => {
    const { tool, applyPose } = makeTool({ forward: [1, 0, 0] })
    tool.onPointerRawDown(100, 100)
    tool.onPointerRawMove(100, 200, 1, { shift: false })
    const [eye] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(eye[0]).toBeLessThan(0)
  })

  it('dragging right turns right (yaw DECREASES — screen-space ground-truthed in cameraWalkMath.test.ts)', () => {
    const { tool, applyPose } = makeTool({ forward: [1, 0, 0] })
    tool.onPointerRawDown(100, 100)
    tool.onPointerRawMove(200, 100, 1, { shift: false })
    const [, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(headingFromForward(forward)).toBeCloseTo(-100 * WALK_TURN_RAD_PER_PIXEL, 9)
  })

  it('Shift: horizontal drag strafes instead of turning, vertical drag changes height instead of forward/back', () => {
    const { tool, applyPose } = makeTool({ forward: [1, 0, 0] })
    tool.onPointerRawDown(100, 100)
    tool.onPointerRawMove(200, 0, 1, { shift: true }) // right+up 100px each
    const [eye, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(headingFromForward(forward)).toBeCloseTo(0, 9) // no turn
    expect(eye[2]).toBeGreaterThan(DEFAULT_EYE_HEIGHT_M) // raised
    // Strafing right while facing +X moves along +/-Y (perpendicular, not forward).
    expect(Math.abs(eye[0])).toBeLessThan(1e-9)
    expect(eye[1]).not.toBeCloseTo(0, 6)
  })

  it('drag distance is measured from the PRESS point, not the last move (speed grows with total drag)', () => {
    const { tool, applyPose } = makeTool({ forward: [1, 0, 0] })
    tool.onPointerRawDown(100, 100)
    tool.onPointerRawMove(100, 50, 1, { shift: false }) // 50px up from press
    const [eyeA] = applyPose.mock.calls.at(-1) as [V3, V3]
    tool.onPointerRawMove(100, 0, 1, { shift: false }) // 100px up from press (not from the last move)
    const [eyeB] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(eyeB[0]).toBeCloseTo(2 * eyeA[0], 6)
  })

  it('releasing commits the drag as the new base; a fresh press continues from there', () => {
    const { tool, applyPose, setEyeHeight } = makeTool({ forward: [1, 0, 0] })
    tool.onPointerRawDown(100, 100)
    tool.onPointerRawMove(100, 0, 1, { shift: false }) // walk forward 100px worth
    const [committedEye] = applyPose.mock.calls.at(-1) as [V3, V3]
    tool.onPointerUp(makeSnap(0, 0, 0), RAY) // commit
    applyPose.mockClear()

    tool.onPointerRawDown(50, 50)
    tool.onPointerRawMove(50, 40, 1, { shift: false }) // another 10px forward
    const [eye] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(eye[0]).toBeCloseTo(committedEye[0] + 10 * WALK_MOVE_M_PER_PIXEL, 6)
    expect(setEyeHeight).not.toHaveBeenCalled() // height never changed (no Shift drag)
  })

  it('a Shift height change persists to the session-shared eye height on release', () => {
    const { tool, setEyeHeight } = makeTool({ forward: [1, 0, 0] })
    tool.onPointerRawDown(100, 100)
    tool.onPointerRawMove(100, 0, 1, { shift: true }) // raise the eye
    tool.onPointerUp(makeSnap(0, 0, 0), RAY)
    expect(setEyeHeight).toHaveBeenCalledWith(DEFAULT_EYE_HEIGHT_M + 100 * WALK_MOVE_M_PER_PIXEL)
  })

  it('typing a height + Enter sets the eye height immediately, mid-idle', () => {
    const { tool, applyPose, setEyeHeight } = makeTool({ eye: [3, 4, DEFAULT_EYE_HEIGHT_M] })
    tool.onKey({ key: '2' } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)
    expect(setEyeHeight).toHaveBeenCalledWith(2)
    const [eye] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(eye).toEqual([3, 4, 2])
  })

  it('Escape cancels and switches back to Select', () => {
    const { tool, onEscapeToSelect } = makeTool()
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(onEscapeToSelect).toHaveBeenCalledTimes(1)
  })

  it('no button held applies nothing', () => {
    const { tool, applyPose } = makeTool()
    tool.onPointerRawMove(100, 100, 0, { shift: false })
    expect(applyPose).not.toHaveBeenCalled()
  })

  // Playtest finding 3: pressing Shift mid-drag (to strafe) after already
  // walking a long way jumped the camera a large, random-looking distance.
  // Root cause: dx/dy are cumulative since the PRESS point (by design — see
  // "drag distance is measured from the PRESS point" above), but Shift
  // switches what they MEAN (forward/turn vs strafe/height) — reinterpreting
  // the SAME already-large accumulated delta under the new mapping produces
  // a jump proportional to how far the drag had already travelled. The fix
  // re-anchors (base pose + press point) the instant the modifier changes.
  describe('Shift toggled mid-gesture re-anchors instead of reinterpreting the stale drag distance', () => {
    it('toggling Shift with the pointer held perfectly still moves the camera not at all', () => {
      const { tool, applyPose } = makeTool({ forward: [1, 0, 0] })
      tool.onPointerRawDown(100, 100)
      // A long walk: 1000px of drag-up is a substantial ~20m forward walk —
      // exactly the kind of "already walking" state finding 3 describes.
      tool.onPointerRawMove(100, -900, 1, { shift: false })
      const [beforeEye] = applyPose.mock.calls.at(-1) as [V3, V3]
      expect(beforeEye[0]).toBeCloseTo(1000 * WALK_MOVE_M_PER_PIXEL, 6) // sanity: a real ~20m walk happened

      // Shift engages with NO further pointer movement (the same xy as the
      // last move) — the live pose must not move AT ALL. Pre-fix, this used
      // to reread the full 1000px of accumulated `dy` as a height delta.
      tool.onPointerRawMove(100, -900, 1, { shift: true })
      const [afterEye] = applyPose.mock.calls.at(-1) as [V3, V3]
      expect(afterEye[0]).toBeCloseTo(beforeEye[0], 9)
      expect(afterEye[1]).toBeCloseTo(beforeEye[1], 9)
      expect(afterEye[2]).toBeCloseTo(beforeEye[2], 9) // no height jump
    })

    it('after the re-anchor, further Shift-drag is proportionate to the NEW travel, not the old accumulated distance', () => {
      const { tool, applyPose } = makeTool({ forward: [1, 0, 0] })
      tool.onPointerRawDown(100, 100)
      tool.onPointerRawMove(100, -900, 1, { shift: false }) // long forward walk
      tool.onPointerRawMove(100, -900, 1, { shift: true }) // Shift engages, no movement yet
      const [anchoredEye] = applyPose.mock.calls.at(-1) as [V3, V3]

      // Now actually drag 50px further while holding Shift.
      tool.onPointerRawMove(100, -950, 1, { shift: true })
      const [eye] = applyPose.mock.calls.at(-1) as [V3, V3]
      expect(eye[2] - anchoredEye[2]).toBeCloseTo(50 * WALK_MOVE_M_PER_PIXEL, 6)
      // Bounded: nowhere near the ~1000px-worth-of-height a stale-anchor
      // reinterpretation would have produced.
      expect(Math.abs(eye[2] - anchoredEye[2])).toBeLessThan(5 * WALK_MOVE_M_PER_PIXEL * 1000)
    })

    it('releasing Shift mid-drag (strafe back to walk/turn) also re-anchors cleanly', () => {
      const { tool, applyPose } = makeTool({ forward: [1, 0, 0] })
      tool.onPointerRawDown(100, 100)
      tool.onPointerRawMove(300, 100, 1, { shift: true }) // strafe right 200px worth
      const [beforeEye] = applyPose.mock.calls.at(-1) as [V3, V3]

      // Release Shift with no further movement — must not jump.
      tool.onPointerRawMove(300, 100, 1, { shift: false })
      const [afterEye, afterForward] = applyPose.mock.calls.at(-1) as [V3, V3]
      expect(afterEye[0]).toBeCloseTo(beforeEye[0], 9)
      expect(afterEye[1]).toBeCloseTo(beforeEye[1], 9)
      expect(afterEye[2]).toBeCloseTo(beforeEye[2], 9)
      expect(headingFromForward(afterForward)).toBeCloseTo(0, 9) // no spurious turn either
    })
  })
})
