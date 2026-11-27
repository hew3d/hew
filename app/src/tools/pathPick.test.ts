import { describe, it, expect } from 'vitest'
import { pickPath, pathHighlight, pathJson } from './pathPick'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const ray: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function facePick(instance: bigint | undefined) {
  return { object: () => 4n, face: () => 2n, instance: () => instance, free: () => {} }
}

describe('pickPath', () => {
  it('a solid face is a face target', () => {
    const scene = { pick_sketch_edge: () => undefined, pick_face: () => facePick(undefined) } as unknown as WasmScene
    expect(pickPath(scene, ray, () => true)).toEqual({
      kind: 'target',
      path: { kind: 'face', objectHandle: 4n, faceHandle: 2n },
    })
  })

  it('a component face refuses', () => {
    const scene = { pick_sketch_edge: () => undefined, pick_face: () => facePick(7n) } as unknown as WasmScene
    expect(pickPath(scene, ray, () => true)).toMatchObject({ kind: 'ineligible-face' })
  })

  it('a face the caller deems ineligible refuses', () => {
    const scene = { pick_sketch_edge: () => undefined, pick_face: () => facePick(undefined) } as unknown as WasmScene
    expect(pickPath(scene, ray, () => false)).toMatchObject({ kind: 'ineligible-face' })
  })

  it('nothing under the ray is none', () => {
    const scene = { pick_sketch_edge: () => undefined, pick_face: () => undefined } as unknown as WasmScene
    expect(pickPath(scene, ray, () => true)).toEqual({ kind: 'none' })
  })

  it('a sketch edge wins over the face behind it and takes its whole run', () => {
    const scene = {
      pick_sketch_edge: () => ({ sketch: () => 1n, edge: () => 5n, free: () => {} }),
      sketch_edge_island: () => 9n,
      sketch_island_edges: () => new BigUint64Array([5n, 6n]),
      pick_face: () => { throw new Error('face must not be consulted') },
    } as unknown as WasmScene
    expect(pickPath(scene, ray, () => true)).toEqual({
      kind: 'target',
      path: { kind: 'edges', sketchHandle: 1n, edgeHandles: [5n, 6n] },
    })
  })
})

describe('pathHighlight and pathJson', () => {
  it('a face loop highlight closes back to its start', () => {
    const scene = { face_boundary: () => new Float32Array([0, 0, 0, 1, 0, 0, 1, 1, 0]) } as unknown as WasmScene
    const h = pathHighlight(scene, { kind: 'face', objectHandle: 4n, faceHandle: 2n })
    expect(h.key).toBe('face:4:2')
    expect(h.points).toHaveLength(3 * 6)
    expect(h.points.slice(-6)).toEqual([1, 1, 0, 0, 0, 0])
  })

  it('a stale handle gives no points', () => {
    const scene = { face_boundary: () => { throw new Error('stale') } } as unknown as WasmScene
    expect(pathHighlight(scene, { kind: 'face', objectHandle: 4n, faceHandle: 2n })).toEqual({ key: 'stale', points: [] })
  })

  it('an edge run keys on its sorted edge set', () => {
    const scene = { sketch_edge_endpoints: () => Float64Array.from([0, 0, 0, 1, 0, 0]) } as unknown as WasmScene
    const a = pathHighlight(scene, { kind: 'edges', sketchHandle: 1n, edgeHandles: [3n, 1n, 2n] })
    const b = pathHighlight(scene, { kind: 'edges', sketchHandle: 1n, edgeHandles: [1n, 2n, 3n] })
    expect(a.key).toBe(b.key)
    expect(a.points).toHaveLength(3 * 6)
  })

  it('writes handles as decimal strings', () => {
    expect(JSON.parse(pathJson({ kind: 'face', objectHandle: 4n, faceHandle: 2n }))).toEqual({ kind: 'face_loop', object: '4', face: '2' })
    expect(JSON.parse(pathJson({ kind: 'edges', sketchHandle: 1n, edgeHandles: [5n, 6n] }))).toEqual({ kind: 'sketch_edges', sketch: '1', edges: ['5', '6'] })
  })
})
