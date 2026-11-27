import { describe, expect, it } from 'vitest'
import { loftSpecJson, nearestVertexIndex, parseStartPoints } from './loftMath'

describe('loftMath', () => {
  it('writes the kernel spec', () => {
    const json = loftSpecJson(
      [
        { kind: 'region', sketch: 1n, region: 2n },
        { kind: 'face', object: 3n, face: 4n },
        { kind: 'vertex', point: [0, 0, 2] },
      ],
      [{ sketch: 5n, edges: [6n, 7n] }],
      [0, 2, 0],
      8,
    )
    expect(JSON.parse(json)).toEqual({
      sources: [
        { region: { sketch: '1', region: '2' } },
        { face: { object: '3', face: '4' } },
        { vertex: [0, 0, 2] },
      ],
      rails: [{ edges: { sketch: '5', edges: ['6', '7'] } }],
      start_offsets: [0, 2, 0],
      stations: 8,
    })
    expect(JSON.parse(loftSpecJson([], [], [], 0)).stations).toBeUndefined()
  })

  it('reads the per-profile start points', () => {
    const lists = parseStartPoints([2, 0, 0, 0, 1, 0, 0, 1, 5, 5, 5])
    expect(lists).toEqual([[[0, 0, 0], [1, 0, 0]], [[5, 5, 5]]])
    expect(parseStartPoints([])).toEqual([])
    expect(parseStartPoints([3, 1, 2, 3])).toEqual([[[1, 2, 3]]]) // truncated
  })

  it('finds the nearest vertex, in the plane when asked', () => {
    const pts: [number, number, number][] = [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]]
    expect(nearestVertexIndex(pts, [0.9, 0.1, 0])).toBe(1)
    expect(nearestVertexIndex(pts, [0.1, 0.9, 5])).toBe(3)
    // Off the plane by a lot, still judged by its in-plane position.
    expect(nearestVertexIndex(pts, [0.9, 0.9, 50], [0, 0, 1])).toBe(2)
    expect(nearestVertexIndex([], [0, 0, 0])).toBe(-1)
  })
})
