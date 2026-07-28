import { describe, it, expect, vi } from 'vitest'
import { PositionCameraTool, POSITION_CAMERA_DRAG_THRESHOLD_M } from './PositionCameraTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { V3 } from './cameraWalkMath'
import { DEFAULT_EYE_HEIGHT_M, headingFromForward } from './cameraWalkMath'

function makeSnap(x: number, y: number, z: number): Snap {
  return { x, y, z, kind: 'ground' }
}

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeTool(overrides: {
  forward?: V3
  eyeHeight?: number
} = {}) {
  const forward = overrides.forward ?? ([1, 0, 0] as V3)
  let eyeHeight = overrides.eyeHeight ?? DEFAULT_EYE_HEIGHT_M
  const applyPose = vi.fn()
  const onAutoSwitch = vi.fn()
  const onEscapeToSelect = vi.fn()
  const onMeasurement = vi.fn()
  const setEyeHeight = vi.fn((h: number) => {
    eyeHeight = h
  })
  const tool = new PositionCameraTool(
    () => forward,
    () => eyeHeight,
    setEyeHeight,
    applyPose,
    onAutoSwitch,
    onEscapeToSelect,
    onMeasurement,
  )
  return { tool, applyPose, onAutoSwitch, onEscapeToSelect, onMeasurement, setEyeHeight, getEyeHeight: () => eyeHeight }
}

describe('PositionCameraTool', () => {
  it('a plain click places the eye eyeHeight above the click point, looking along the pre-click heading', () => {
    const { tool, applyPose, onAutoSwitch } = makeTool({ forward: [0, 1, 0] })
    tool.onPointerDown(makeSnap(3, 4, 0), RAY)
    tool.onPointerUp(makeSnap(3, 4, 0), RAY) // negligible movement from press -> a click
    const lastCall = applyPose.mock.calls.at(-1)
    expect(lastCall).toBeDefined()
    const [eye, forward] = lastCall as [V3, V3]
    expect(eye[0]).toBeCloseTo(3, 9)
    expect(eye[1]).toBeCloseTo(4, 9)
    expect(eye[2]).toBeCloseTo(DEFAULT_EYE_HEIGHT_M, 9)
    expect(forward[2]).toBeCloseTo(0, 9) // horizontal
    expect(headingFromForward(forward)).toBeCloseTo(Math.PI / 2, 9) // pre-click heading (+Y)
    expect(onAutoSwitch).toHaveBeenCalledTimes(1)
  })

  it('a drag past the threshold places the eye at the press point, looking toward the release point', () => {
    const { tool, applyPose, onAutoSwitch } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), RAY)
    tool.onPointerUp(makeSnap(10, 0, 0), RAY) // well past the drag threshold
    const [eye, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(eye).toEqual([0, 0, DEFAULT_EYE_HEIGHT_M])
    // Looking toward (10,0,0) from (0,0,eyeHeight): mostly +X, slightly down.
    expect(forward[0]).toBeGreaterThan(0.9)
    expect(forward[2]).toBeLessThan(0)
    expect(onAutoSwitch).toHaveBeenCalledTimes(1)
  })

  it('live-previews the drag pose on every pointer move while pressed', () => {
    const { tool, applyPose } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), RAY)
    applyPose.mockClear()
    tool.onPointerMove(makeSnap(5, 0, 0), RAY)
    expect(applyPose).toHaveBeenCalledTimes(1)
    const [eye] = applyPose.mock.calls[0] as [V3, V3]
    expect(eye).toEqual([0, 0, DEFAULT_EYE_HEIGHT_M])
  })

  it('a sub-threshold release still resolves to the CLICK pose, not a straight-down drag look', () => {
    const { tool, applyPose } = makeTool({ forward: [0, 1, 0] })
    tool.onPointerDown(makeSnap(0, 0, 0), RAY)
    // Tiny jitter, well under POSITION_CAMERA_DRAG_THRESHOLD_M.
    tool.onPointerUp(makeSnap(POSITION_CAMERA_DRAG_THRESHOLD_M / 10, 0, 0), RAY)
    const [, forward] = applyPose.mock.calls.at(-1) as [V3, V3]
    expect(forward[2]).toBeCloseTo(0, 6) // horizontal, not a drag look
    expect(headingFromForward(forward)).toBeCloseTo(Math.PI / 2, 6)
  })

  it('does nothing on pointer down with no snap', () => {
    const { tool, applyPose } = makeTool()
    tool.onPointerDown(null, RAY)
    expect(applyPose).not.toHaveBeenCalled()
  })

  it('typing a height + Enter sets the session eye height', () => {
    const { tool, setEyeHeight, onMeasurement } = makeTool()
    tool.onKey({ key: '2' } as KeyboardEvent)
    tool.onKey({ key: '.' } as KeyboardEvent)
    tool.onKey({ key: '1' } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)
    expect(setEyeHeight).toHaveBeenCalledWith(2.1)
    expect(onMeasurement).toHaveBeenLastCalledWith('')
  })

  it('Escape cancels without setting an eye height, and switches back to Select (design §4 / user guide: "Esc returns to the Select tool from any of the three")', () => {
    const { tool, setEyeHeight, onMeasurement, onEscapeToSelect } = makeTool()
    tool.onKey({ key: '5' } as KeyboardEvent)
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(setEyeHeight).not.toHaveBeenCalled()
    expect(onMeasurement).toHaveBeenLastCalledWith('')
    expect(tool.capturingInput()).toBe(false)
    expect(onEscapeToSelect).toHaveBeenCalledTimes(1)
  })

  it('capturesKey lets digits through unconditionally but gates unit letters on a started buffer (never eats a bare tool-switch letter)', () => {
    const { tool } = makeTool()
    expect(tool.capturesKey('5')).toBe(true)
    expect(tool.capturesKey('m')).toBe(false)
    tool.onKey({ key: '5' } as KeyboardEvent)
    expect(tool.capturesKey('m')).toBe(true)
  })
})
