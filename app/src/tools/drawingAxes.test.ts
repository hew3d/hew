import { describe, it, expect, vi } from 'vitest'
import { getDrawingAxes, isWorldIdentity, WORLD_DRAWING_AXES } from './drawingAxes'

function makeWasmScene(flat: number[]) {
  return { axes: vi.fn(() => new Float64Array(flat)) }
}

describe('getDrawingAxes', () => {
  it('reshapes the flat 12-float buffer into origin/x/y/z', () => {
    const scene = makeWasmScene([1, 2, 3, 0, 1, 0, -1, 0, 0, 0, 0, 1])
    const frame = getDrawingAxes(scene as never)
    expect(frame.origin).toEqual([1, 2, 3])
    expect(frame.x).toEqual([0, 1, 0])
    expect(frame.y).toEqual([-1, 0, 0])
    expect(frame.z).toEqual([0, 0, 1])
  })

  it('reshapes the world-identity buffer to WORLD_DRAWING_AXES', () => {
    const scene = makeWasmScene([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])
    expect(getDrawingAxes(scene as never)).toEqual(WORLD_DRAWING_AXES)
  })
})

describe('isWorldIdentity', () => {
  it('is true for the exact world-identity frame', () => {
    expect(isWorldIdentity(WORLD_DRAWING_AXES)).toBe(true)
  })

  it('is true within float tolerance of world identity', () => {
    expect(isWorldIdentity({
      origin: [1e-12, -1e-12, 0],
      x: [1, 1e-13, 0],
      y: [0, 1, 0],
      z: [0, 0, 1],
    })).toBe(true)
  })

  it('is false for a translated origin', () => {
    expect(isWorldIdentity({ origin: [1, 0, 0], x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] })).toBe(false)
  })

  it('is false for a rotated frame (axes swapped)', () => {
    expect(isWorldIdentity({ origin: [0, 0, 0], x: [0, 1, 0], y: [-1, 0, 0], z: [0, 0, 1] })).toBe(false)
  })
})
