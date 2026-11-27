import { describe, expect, it } from 'vitest'
import {
  chainPolyline,
  closestOnPolyline,
  emptyGripState,
  gripStateIsActive,
  handlesForSpec,
  lineParamNearRay,
  movedHandle,
  pointAtLength,
  polylineLength,
  withHandle,
  withStop,  sameGrip,
} from './filletGrips'

describe('chain polyline', () => {
  it('orders segments end to end whatever their own direction', () => {
    // Three edges along +x, the middle one stored backwards.
    const segs = new Float32Array([0, 0, 0, 1, 0, 0, 2, 0, 0, 1, 0, 0, 2, 0, 0, 3, 0, 0])
    const pts = chainPolyline(segs)
    expect(pts).toEqual([
      [0, 0, 0],
      [1, 0, 0],
      [2, 0, 0],
      [3, 0, 0],
    ])
    expect(polylineLength(pts)).toBe(3)
  })

  it('starts from the right end when the first segment is stored backwards', () => {
    const pts = chainPolyline(new Float32Array([1, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0]))
    expect(pts[0]).toEqual([0, 0, 0])
    expect(pts[2]).toEqual([1, 1, 0])
  })

  it('drops the repeated point of a closed ring', () => {
    const ring = new Float32Array([0, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 0, 0, 0, 0])
    expect(chainPolyline(ring)).toHaveLength(3)
  })

  it('finds points by arc length and by pointer', () => {
    const pts = chainPolyline(new Float32Array([0, 0, 0, 2, 0, 0]))
    expect(pointAtLength(pts, 0.5)).toEqual([0.5, 0, 0])
    expect(pointAtLength(pts, 9)).toEqual([2, 0, 0])
    const hit = closestOnPolyline(pts, [1.2, 0.3, 5], [0, 0, -1])
    expect(hit?.s).toBeCloseTo(1.2, 9)
    expect(hit?.distance).toBeCloseTo(0.3, 9)
    expect(closestOnPolyline([], [0, 0, 0], [0, 0, 1])).toBeNull()
  })

  it('reads how far along an axis the pointer has dragged', () => {
    // The bisector rises along +z from the origin; the ray is looking along -y
    // from y = 5 at z = 0.4.
    const t = lineParamNearRay([0, 0, 0], [0, 0, 1], [0, 5, 0.4], [0, -1, 0])
    expect(t).toBeCloseTo(0.4, 9)
    expect(lineParamNearRay([0, 0, 0], [0, 0, 1], [0, 0, 5], [0, 0, -1])).toBeNull()
  })
})

describe('grip identity', () => {
  it('matches grips by kind and end or index', () => {
    expect(sameGrip({ kind: 'radius' }, { kind: 'radius' })).toBe(true)
    expect(sameGrip({ kind: 'stop', end: 'start' }, { kind: 'stop', end: 'end' })).toBe(false)
    expect(sameGrip({ kind: 'handlePos', index: 1 }, { kind: 'handlePos', index: 1 })).toBe(true)
    expect(sameGrip({ kind: 'handlePos', index: 1 }, { kind: 'handleRadius', index: 1 })).toBe(false)
    expect(sameGrip(null, { kind: 'radius' })).toBe(false)
    expect(sameGrip(null, null)).toBe(true)
  })
})

describe('stops and handles', () => {
  it('sets a stop from the pointer and keeps the stops apart', () => {
    let g = emptyGripState()
    expect(gripStateIsActive(g)).toBe(false)
    g = withStop(g, 'start', 0.2, 1)
    expect(g.stops).toEqual({ start: 0.2, end: 0 })
    g = withStop(g, 'end', 0.7, 1)
    expect(g.stops.end).toBeCloseTo(0.3, 9)
    // The end stop cannot swallow the start stop.
    g = withStop(g, 'end', 0.1, 1)
    expect(g.stops.end).toBeLessThan(0.8)
    expect(gripStateIsActive(g)).toBe(true)
    // Dragging back to the end clears it.
    expect(withStop(g, 'start', 0, 1).stops.start).toBe(0)
  })

  it('adds, replaces and moves handles in order', () => {
    let g = emptyGripState()
    g = withHandle(g, 0.5, 0.02, 1e-6)
    g = withHandle(g, 0, 0.01, 1e-6)
    g = withHandle(g, 0.5, 0.03, 1e-6)
    expect(g.handles).toEqual([
      { at: 0, radius: 0.01 },
      { at: 0.5, radius: 0.03 },
    ])
    g = movedHandle(g, 0, 2, 1)
    expect(g.handles.map((h) => h.at)).toEqual([0.5, 1])
    expect(handlesForSpec(emptyGripState())).toBeUndefined()
    expect(handlesForSpec(g)).toHaveLength(2)
  })
})
