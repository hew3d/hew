/**
 * transformSelection — sketch-selection routing for the transform tools.
 *
 * Pins the fixes for the "cannot rotate any shape up 90 degrees" defect
 * class: a selected edge/curve transforms its ISLAND (never a silent
 * no-op), islands covering a whole sketch fold into one handle-stable
 * whole-sketch bake, and a strict subset routes through the per-island
 * kernel op with an all-first validation pass.
 */
import { describe, it, expect, vi } from 'vitest'
import {
  commitSelectionTransform,
  planSketchTransforms,
  resolveSketchIsland,
} from './transformSelection'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'

const AFFINE = new Float64Array([1, 0, 0, 5, 0, 1, 0, 0, 0, 0, 1, 0])

interface FakeOpts {
  /** island ids per sketch handle (decimal string keys) */
  islands?: Record<string, bigint[]>
  /** edge id -> island id */
  edgeIslands?: Record<string, bigint>
  canTransformIsland?: boolean
}

function makeScene(opts: FakeOpts = {}) {
  const calls: { name: string; args: unknown[] }[] = []
  const scene = {
    sketch_island_ids: vi.fn((sketch: bigint) => opts.islands?.[String(sketch)] ?? []),
    sketch_edge_island: vi.fn(
      (_sketch: bigint, edge: bigint) => opts.edgeIslands?.[String(edge)],
    ),
    can_transform_sketch_island: vi.fn(() => opts.canTransformIsland ?? true),
    transform_sketch_island: vi.fn((...args: unknown[]) => {
      calls.push({ name: 'transform_sketch_island', args })
    }),
    transform_selection: vi.fn((...args: unknown[]) => {
      calls.push({ name: 'transform_selection', args })
    }),
  }
  return { scene: scene as unknown as WasmScene, raw: scene, calls }
}

describe('resolveSketchIsland', () => {
  it('maps an edge and a curve to their owning island', () => {
    const { scene } = makeScene({ edgeIslands: { '7': 40n } })
    const edge: NodeRef = { kind: 'sketch-edge', id: 7n, sketch: 3n }
    const curve: NodeRef = { kind: 'sketch-curve', id: 7n, sketch: 3n }
    expect(resolveSketchIsland(scene, edge)).toEqual({ sketch: 3n, island: 40n })
    expect(resolveSketchIsland(scene, curve)).toEqual({ sketch: 3n, island: 40n })
  })

  it('returns null for a stale edge (pruned selection)', () => {
    const { scene } = makeScene()
    expect(
      resolveSketchIsland(scene, { kind: 'sketch-edge', id: 9n, sketch: 3n }),
    ).toBeNull()
  })
})

describe('planSketchTransforms', () => {
  it('folds islands covering the whole sketch into a whole-sketch bake', () => {
    const { scene } = makeScene({ islands: { '3': [40n] } })
    const plan = planSketchTransforms(scene, [
      { kind: 'sketch-island', id: 40n, sketch: 3n },
    ])
    expect(plan.sketches).toEqual([3n])
    expect(plan.islands).toEqual([])
  })

  it('keeps a strict subset of islands at island granularity', () => {
    const { scene } = makeScene({ islands: { '3': [40n, 41n] } })
    const plan = planSketchTransforms(scene, [
      { kind: 'sketch-island', id: 40n, sketch: 3n },
    ])
    expect(plan.sketches).toEqual([])
    expect(plan.islands).toEqual([{ sketch: 3n, island: 40n }])
  })

  it('dedupes an edge selected alongside its own island', () => {
    const { scene } = makeScene({
      islands: { '3': [40n, 41n] },
      edgeIslands: { '7': 40n },
    })
    const plan = planSketchTransforms(scene, [
      { kind: 'sketch-island', id: 40n, sketch: 3n },
      { kind: 'sketch-edge', id: 7n, sketch: 3n },
    ])
    expect(plan.islands).toEqual([{ sketch: 3n, island: 40n }])
  })

  it('a whole-sketch selection absorbs its own islands', () => {
    const { scene } = makeScene({ islands: { '3': [40n, 41n] } })
    const plan = planSketchTransforms(scene, [
      { kind: 'sketch', id: 3n },
      { kind: 'sketch-island', id: 40n, sketch: 3n },
    ])
    expect(plan.sketches).toEqual([3n])
    expect(plan.islands).toEqual([])
  })
})

describe('commitSelectionTransform', () => {
  it('transforms the island of a selected open-chain edge (no silent no-op)', () => {
    const { scene, raw } = makeScene({
      islands: { '3': [40n, 41n] },
      edgeIslands: { '7': 40n },
    })
    commitSelectionTransform(scene, [{ kind: 'sketch-edge', id: 7n, sketch: 3n }], AFFINE)
    expect(raw.transform_sketch_island).toHaveBeenCalledWith(3n, 40n, AFFINE)
    expect(raw.transform_selection).not.toHaveBeenCalled()
  })

  it('routes a sole-island selection through the whole-sketch path', () => {
    const { scene, raw } = makeScene({ islands: { '3': [40n] } })
    commitSelectionTransform(
      scene,
      [{ kind: 'sketch-island', id: 40n, sketch: 3n }],
      AFFINE,
    )
    expect(raw.transform_sketch_island).not.toHaveBeenCalled()
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    const sketches = raw.transform_selection.mock.calls[0][2] as BigUint64Array
    expect([...sketches]).toEqual([3n])
  })

  it('validates every subset island before committing any', () => {
    const { scene, raw } = makeScene({
      islands: { '3': [40n, 41n, 42n] },
      canTransformIsland: false,
    })
    expect(() =>
      commitSelectionTransform(
        scene,
        [
          { kind: 'sketch-island', id: 40n, sketch: 3n },
          { kind: 'sketch-island', id: 41n, sketch: 3n },
        ],
        AFFINE,
      ),
    ).toThrow(/WouldRetopologize/)
    expect(raw.transform_sketch_island).not.toHaveBeenCalled()
    expect(raw.transform_selection).not.toHaveBeenCalled()
  })

  it('commits objects and sketch geometry together', () => {
    const { scene, raw } = makeScene({ islands: { '3': [40n, 41n] } })
    commitSelectionTransform(
      scene,
      [
        { kind: 'object', id: 9n },
        { kind: 'sketch-island', id: 40n, sketch: 3n },
      ],
      AFFINE,
    )
    expect(raw.transform_sketch_island).toHaveBeenCalledWith(3n, 40n, AFFINE)
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    const ids = raw.transform_selection.mock.calls[0][1] as BigUint64Array
    expect([...ids]).toEqual([9n])
  })

  it('skips a stale edge like any pruned handle instead of throwing', () => {
    const { scene, raw } = makeScene({ islands: { '3': [40n] } })
    commitSelectionTransform(scene, [{ kind: 'sketch-edge', id: 99n, sketch: 3n }], AFFINE)
    expect(raw.transform_sketch_island).not.toHaveBeenCalled()
    expect(raw.transform_selection).not.toHaveBeenCalled()
  })
})
