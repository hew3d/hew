import { describe, it, expect, vi } from 'vitest'
import { LookAroundTool, LOOK_AROUND_RAD_PER_PIXEL } from './LookAroundTool'
import type { V3 } from './cameraWalkMath'
import { DEFAULT_EYE_HEIGHT_M, PITCH_CLAMP_DEG, headingFromForward } from './cameraWalkMath'

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
  const tool = new LookAroundTool(
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

describe('LookAroundTool', () => {
  it('onPointerRawDown seeds the baseline immediately, so even a SINGLE subsequent move (a fast drag delivered as one coalesced event) still rotates by the full press-to-there distance', () => {
    const { tool, applyPose } = makeTool()
    tool.onPointerRawDown(100, 100)
    tool.onPointerRawMove(200, 100, 1, { shift: false }) // one move, 100px right of the press
    const [, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    // Right drag turns right (yaw DECREASES) — screen-space ground-truthed
    // in cameraWalkMath.test.ts.
    expect(headingFromForward(forward)).toBeCloseTo(-100 * LOOK_AROUND_RAD_PER_PIXEL, 9)
  })

  it('a drag with the left button held yaws the view', () => {
    const { tool, applyPose } = makeTool()
    tool.onPointerRawMove(100, 100, 1, { shift: false }) // first sample: seeds the baseline, no pose change
    expect(applyPose).not.toHaveBeenCalled()
    tool.onPointerRawMove(200, 100, 1, { shift: false }) // drag right 100px
    const [eye, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(eye).toEqual([0, 0, DEFAULT_EYE_HEIGHT_M])
    expect(headingFromForward(forward)).toBeCloseTo(-100 * LOOK_AROUND_RAD_PER_PIXEL, 9)
  })

  it('dragging up looks up (increases pitch)', () => {
    const { tool, applyPose } = makeTool()
    tool.onPointerRawMove(100, 100, 1, { shift: false })
    tool.onPointerRawMove(100, 0, 1, { shift: false }) // drag up 100px
    const [, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(forward[2]).toBeGreaterThan(0)
  })

  it('without the left button, no drag is applied and the baseline resets', () => {
    const { tool, applyPose } = makeTool()
    tool.onPointerRawMove(100, 100, 0, { shift: false }) // hover, no button
    tool.onPointerRawMove(300, 100, 0, { shift: false })
    expect(applyPose).not.toHaveBeenCalled()
  })

  it('releasing and re-pressing starts a fresh delta baseline (no jump from the gap)', () => {
    const { tool, applyPose } = makeTool()
    tool.onPointerRawMove(100, 100, 1, { shift: false })
    tool.onPointerRawMove(200, 100, 1, { shift: false }) // yaw += 100px worth
    applyPose.mockClear()
    tool.onPointerRawMove(200, 100, 0, { shift: false }) // release (button up)
    tool.onPointerRawMove(900, 100, 1, { shift: false }) // big jump while up, THEN press here
    // The jump-while-up sample only seeds the new baseline; no pose change yet.
    expect(applyPose).not.toHaveBeenCalled()
    tool.onPointerRawMove(910, 100, 1, { shift: false }) // small drag from the new baseline
    const [, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    // Only a 10px drag's worth of additional yaw, not (910-200)px.
    const totalYaw = headingFromForward(forward)
    expect(totalYaw).toBeCloseTo(-(100 * LOOK_AROUND_RAD_PER_PIXEL + 10 * LOOK_AROUND_RAD_PER_PIXEL), 6)
  })

  it('pitch is clamped to ±PITCH_CLAMP_DEG regardless of how far the drag goes', () => {
    const { tool, applyPose } = makeTool()
    tool.onPointerRawMove(0, 1_000_000, 1, { shift: false })
    tool.onPointerRawMove(0, 0, 1, { shift: false }) // drag up by a million pixels
    const [, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    const clampSin = Math.sin((PITCH_CLAMP_DEG * Math.PI) / 180)
    expect(forward[2]).toBeCloseTo(clampSin, 6)
  })

  it('typing a height + Enter re-heights the eye in place (same x/y)', () => {
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

  it('seeds yaw/pitch from the pre-activation forward at construction', () => {
    const { tool, applyPose } = makeTool({ forward: [0, 1, 0] }) // pre-activation heading +Y
    tool.onPointerRawMove(100, 100, 1, { shift: false })
    tool.onPointerRawMove(101, 100, 1, { shift: false }) // tiny nudge
    const [, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    // Still close to +Y (only a 1px nudge away), not the default +X.
    expect(headingFromForward(forward)).toBeCloseTo(Math.PI / 2, 2)
  })
})
