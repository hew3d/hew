import { describe, it, expect } from 'vitest'
import { pickEdgePath, edgePathPolyline, loopPolyline, polylineSegments, type PolylinePath } from './edgePathPick'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const ray: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

describe('polylineSegments', () => {
  it('an open path has n-1 pairs, a closed one has n', () => {
    const open: PolylinePath = { points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]], closed: false }
    expect(polylineSegments(open)).toHaveLength(2 * 6)
    expect(polylineSegments({ ...open, closed: true })).toHaveLength(3 * 6)
  })
})

/** A sketch of straight edges keyed by name; `island` lists the picked run. */
function sketchScene(edges: Record<string, [number, number, number, number, number, number]>, island: string[]): WasmScene {
  const ids = Object.keys(edges)
  return {
    pick_sketch_edge: () => ({ sketch: () => 1n, edge: () => BigInt(ids.indexOf(island[0]) + 1), free: () => {} }),
    sketch_edge_island: () => 99n,
    sketch_island_edges: () => new BigUint64Array(island.map((k) => BigInt(ids.indexOf(k) + 1))),
    sketch_edge_endpoints: (_s: bigint, e: bigint) => Float64Array.from(edges[ids[Number(e) - 1]]),
  } as unknown as WasmScene
}

describe('pickEdgePath + edgePathPolyline', () => {
  it('a chain of drawn lines becomes one ordered path, end to end', () => {
    const scene = sketchScene({ a: [0, 0, 0, 1, 0, 0], b: [1, 0, 0, 1, 1, 0] }, ['a', 'b'])
    const picked = pickEdgePath(scene, ray)
    expect(picked?.edgeHandles).toEqual([1n, 2n])
    const poly = edgePathPolyline(scene, picked!.sketchHandle, picked!.edgeHandles)
    expect(poly?.closed).toBe(false)
    expect(poly?.points).toEqual([[0, 0, 0], [1, 0, 0], [1, 1, 0]])
  })

  it('a chain stored out of order and flipped still reads end to end', () => {
    const scene = sketchScene({ a: [1, 1, 0, 1, 0, 0], b: [0, 0, 0, 1, 0, 0] }, ['a', 'b'])
    const picked = pickEdgePath(scene, ray)!
    const poly = edgePathPolyline(scene, picked.sketchHandle, picked.edgeHandles)
    expect(poly?.closed).toBe(false)
    expect(poly?.points).toHaveLength(3)
  })

  it('a branch does not chain', () => {
    const scene = sketchScene(
      { a: [0, 0, 0, 1, 0, 0], b: [0, 0, 0, 0, 1, 0], c: [0, 0, 0, -1, 0, 0] },
      ['a', 'b', 'c'],
    )
    const picked = pickEdgePath(scene, ray)!
    expect(edgePathPolyline(scene, picked.sketchHandle, picked.edgeHandles)).toBeNull()
  })

  it('a closed run drops the repeated first point', () => {
    const scene = sketchScene(
      { a: [0, 0, 0, 1, 0, 0], b: [1, 0, 0, 1, 1, 0], c: [1, 1, 0, 0, 0, 0] },
      ['a', 'b', 'c'],
    )
    const picked = pickEdgePath(scene, ray)!
    const poly = edgePathPolyline(scene, picked.sketchHandle, picked.edgeHandles)
    expect(poly?.closed).toBe(true)
    expect(poly?.points).toHaveLength(3)
  })

  it('a stale endpoint lookup is no path', () => {
    const scene = {
      sketch_edge_endpoints: () => { throw new Error('stale') },
    } as unknown as WasmScene
    expect(edgePathPolyline(scene, 1n, [1n])).toBeNull()
  })

  it('a stale island falls back to the one picked edge', () => {
    const scene = {
      pick_sketch_edge: () => ({ sketch: () => 1n, edge: () => 5n, free: () => {} }),
      sketch_edge_island: () => { throw new Error('stale') },
    } as unknown as WasmScene
    expect(pickEdgePath(scene, ray)).toEqual({ sketchHandle: 1n, edgeHandles: [5n] })
  })

  it('no sketch edge under the ray is null', () => {
    const scene = { pick_sketch_edge: () => undefined } as unknown as WasmScene
    expect(pickEdgePath(scene, ray)).toBeNull()
  })
})

describe('loopPolyline', () => {
  it('a face loop is a closed path', () => {
    const poly = loopPolyline([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0])
    expect(poly?.closed).toBe(true)
    expect(poly?.points).toHaveLength(4)
  })

  it('fewer than three points is no loop', () => {
    expect(loopPolyline([0, 0, 0, 1, 0, 0])).toBeNull()
  })
})
