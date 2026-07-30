import { describe, expect, it } from 'vitest'
import { MAX_FOV_DEG, MIN_FOV_DEG } from './cameraRig'
import {
  FOV_DRAG_K,
  FOV_WHEEL_K,
  beginFovDrag,
  decideFovDragMode,
  fovAfterWheel,
  fovDragValue,
} from './fovDrag'

describe('fovDragValue — direction (camera-playtest2.md §3, sign verified against this build\'s real dolly)', () => {
  it('dragging UP (decreasing screen Y) narrows the fov, matching the dolly-in convention', () => {
    const state = beginFovDrag(500, 45)
    expect(fovDragValue(state, 100)).toBeLessThan(45)
  })

  it('dragging DOWN (increasing screen Y) widens the fov, matching the dolly-out convention', () => {
    const state = beginFovDrag(500, 45)
    expect(fovDragValue(state, 900)).toBeGreaterThan(45)
  })

  it('no travel from the press leaves the fov unchanged', () => {
    const state = beginFovDrag(500, 45)
    expect(fovDragValue(state, 500)).toBeCloseTo(45, 9)
  })
})

describe('fovDragValue — multiplicative shape, pinned to the design\'s "400px ~= 3x" anchor', () => {
  it('a 400px downward drag multiplies the base fov by ~3', () => {
    const state = beginFovDrag(0, 10)
    expect(fovDragValue(state, 400)).toBeCloseTo(30, 9)
  })

  it('a 400px upward drag divides the base fov by ~3 (the exact inverse)', () => {
    const state = beginFovDrag(0, 10)
    expect(fovDragValue(state, -400)).toBeCloseTo(10 / 3, 9)
  })

  it('FOV_DRAG_K is exactly the constant that derivation implies (400 / ln 3)', () => {
    expect(FOV_DRAG_K).toBeCloseTo(400 / Math.log(3), 12)
  })

  it('doubling the travel squares the multiplier — exponential in travel, not linear', () => {
    const base = 5
    const state = beginFovDrag(0, base)
    const ratioAt200 = fovDragValue(state, 200) / base
    const ratioAt400 = fovDragValue(state, 400) / base
    expect(ratioAt400).toBeCloseTo(ratioAt200 * ratioAt200, 9)
  })
})

describe('fovDragValue — clamp ends', () => {
  it('clamps to MAX_FOV_DEG on a large downward (widening) drag', () => {
    const state = beginFovDrag(0, 45)
    expect(fovDragValue(state, 100_000)).toBe(MAX_FOV_DEG)
  })

  it('clamps to MIN_FOV_DEG on a large upward (narrowing) drag', () => {
    const state = beginFovDrag(0, 45)
    expect(fovDragValue(state, -100_000)).toBe(MIN_FOV_DEG)
  })
})

describe('fovAfterWheel — same sign as the drag, smaller steps', () => {
  it('a positive deltaY (scroll down/away) widens the fov, matching the drag\'s down-widens sign', () => {
    expect(fovAfterWheel(45, 100)).toBeGreaterThan(45)
  })

  it('a negative deltaY (scroll up/toward) narrows the fov, matching the drag\'s up-narrows sign', () => {
    expect(fovAfterWheel(45, -100)).toBeLessThan(45)
  })

  it('one wheel tick moves the fov far less than the same-magnitude drag travel — "smaller steps"', () => {
    const wheelDelta = Math.abs(fovAfterWheel(45, 100) - 45)
    const dragDelta = Math.abs(fovDragValue(beginFovDrag(0, 45), 100) - 45)
    expect(wheelDelta).toBeLessThan(dragDelta)
  })

  it('FOV_WHEEL_K is a fixed multiple of FOV_DRAG_K (10x less sensitive)', () => {
    expect(FOV_WHEEL_K).toBeCloseTo(FOV_DRAG_K * 10, 9)
  })

  it('clamps to MAX_FOV_DEG / MIN_FOV_DEG on extreme wheel deltas', () => {
    expect(fovAfterWheel(45, 1_000_000)).toBe(MAX_FOV_DEG)
    expect(fovAfterWheel(45, -1_000_000)).toBe(MIN_FOV_DEG)
  })
})

describe('decideFovDragMode — the fixed-at-press mode decision', () => {
  it('Shift held at press decides fov', () => {
    expect(decideFovDragMode(true)).toBe('fov')
  })

  it('Shift NOT held at press decides dolly', () => {
    expect(decideFovDragMode(false)).toBe('dolly')
  })
})
