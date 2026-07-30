import { describe, it, expect } from 'vitest'
import { inferenceAxisName, inferenceCssColor, KIND_CSS_COLOR } from './inferenceColor'
import type { InferenceInfo } from './Viewport'
import type { DrawingAxes } from '../tools/drawingAxes'

/** A non-identity frame (tool-parity §4) — red/green swapped-and-flipped
 *  relative to world, blue unchanged. Mirrors the fixture MoveTool.test.ts /
 *  RotateTool.test.ts use for their own moved-frame regressions. */
const ROTATED_FRAME: DrawingAxes = {
  origin: [0, 0, 0],
  x: [0, 1, 0],
  y: [-1, 0, 0],
  z: [0, 0, 1],
}

function makeInfo(overrides: Partial<InferenceInfo> = {}): InferenceInfo {
  return { kind: 'on-axis', screenX: 0, screenY: 0, ...overrides }
}

describe('inferenceAxisName', () => {
  it('null when the info carries no direction', () => {
    expect(inferenceAxisName(makeInfo())).toBeNull()
  })

  it('names literal world X as "red" when no frame is attached (defaults to world identity)', () => {
    expect(inferenceAxisName(makeInfo({ direction: [1, 0, 0] }))).toBe('red')
  })

  it('names literal world Z as "blue"', () => {
    expect(inferenceAxisName(makeInfo({ direction: [0, 0, 1] }))).toBe('blue')
  })

  it('null for an off-axis direction', () => {
    const d = Math.SQRT1_2
    expect(inferenceAxisName(makeInfo({ direction: [d, d, 0] }))).toBeNull()
  })

  // ── Movable drawing axes (tool-parity §4) ──────────────────────────────
  it('a rotated frame\'s red (X) axis direction still names "red", not a miss or the wrong color', () => {
    // The frame's red axis is world [0,1,0] — under the un-fixed (world-only)
    // primitive this direction is either "green" (it IS literal world Y) or,
    // depending on tolerance, unmatched — never correctly "red".
    const info = makeInfo({ direction: ROTATED_FRAME.x, frame: ROTATED_FRAME })
    expect(inferenceAxisName(info)).toBe('red')
  })

  it('the SAME world direction [0,1,0] names "green" when no frame is attached (world path unchanged)', () => {
    const info = makeInfo({ direction: [0, 1, 0] })
    expect(inferenceAxisName(info)).toBe('green')
  })
})

describe('inferenceCssColor', () => {
  it('an on-axis inference resolves to the --axis-* CSS var for its frame-relative axis', () => {
    const info = makeInfo({ kind: 'on-axis', direction: ROTATED_FRAME.x, frame: ROTATED_FRAME })
    expect(inferenceCssColor(info)).toBe('var(--axis-red)')
  })

  it('a non-axis kind falls back to its KIND_CSS_COLOR entry regardless of frame', () => {
    const info = makeInfo({ kind: 'endpoint', direction: ROTATED_FRAME.x, frame: ROTATED_FRAME })
    expect(inferenceCssColor(info)).toBe(KIND_CSS_COLOR.endpoint)
  })
})
