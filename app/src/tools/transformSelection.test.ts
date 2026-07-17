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
  duplicateSketchSelection,
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

// ---------------------------------------------------------------------------
// duplicateSketchSelection — Move+Alt's sketch copy path (playtest: "you can
// Move a Sketch but you can't Copy a Sketch"). The copy is a translated
// replay through the ordinary drawing surface: one gesture per sketch (one
// undo step), curve chains re-bracketed with their translated analytic
// definition so a copied circle stays a true circle.
// ---------------------------------------------------------------------------

interface ReplayFake {
  scene: WasmScene
  raw: Record<string, ReturnType<typeof vi.fn>>
  log: string[]
}

/** A one-sketch fake with two islands: 40 (a 2-edge curve chain, circle
 * geom center (1,2,0) r 0.5) and 41 (one plain edge). Ground plane. */
function makeReplayScene(opts: { failOnAdd?: boolean } = {}): ReplayFake {
  const log: string[] = []
  let nextEdge = 500n
  const endpoints: Record<string, number[]> = {
    '10': [1.5, 2, 0, 1, 2.5, 0],
    '11': [1, 2.5, 0, 0.5, 2, 0],
    '20': [5, 5, 0, 6, 5, 0],
  }
  const raw = {
    sketch_island_ids: vi.fn(() => [40n, 41n]),
    sketch_island_edges: vi.fn((_s: bigint, island: bigint) =>
      island === 40n ? [10n, 11n] : island === 41n ? [20n] : [],
    ),
    sketch_edge_island: vi.fn((_s: bigint, edge: bigint) =>
      edge >= 500n ? 60n : edge === 20n ? 41n : 40n,
    ),
    sketch_edge_endpoints: vi.fn((_s: bigint, e: bigint) => endpoints[String(e)]),
    sketch_edge_curve: vi.fn((_s: bigint, e: bigint) => (e === 10n || e === 11n ? 7n : undefined)),
    sketch_curve_geom: vi.fn(() => [1, 2, 0, 0.5]),
    sketch_plane: vi.fn(() => [0, 0, 0, 0, 0, 1]),
    sketch_begin_gesture: vi.fn(() => log.push('begin_gesture')),
    sketch_end_gesture: vi.fn(() => log.push('end_gesture')),
    sketch_cancel_gesture: vi.fn(() => log.push('cancel_gesture')),
    sketch_begin_curve: vi.fn(() => { log.push('begin_curve'); return 8n }),
    sketch_begin_curve_with: vi.fn((_s: bigint, cx: number, cy: number, cz: number, r: number) => {
      log.push(`begin_curve_with(${cx},${cy},${cz},${r})`)
      return 8n
    }),
    sketch_end_curve: vi.fn(() => log.push('end_curve')),
    sketch_add_segment: vi.fn((_s: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
      if (opts.failOnAdd === true) throw new Error('WouldRetopologize: nope')
      log.push(`add(${ax},${ay},${az} -> ${bx},${by},${bz})`)
      const id = nextEdge++
      return { new_edges: () => [id], free: () => { /* no-op */ } }
    }),
  }
  return { scene: raw as unknown as WasmScene, raw, log }
}

describe('duplicateSketchSelection', () => {
  it('replays a subset island translated, inside one gesture, with the curve re-bracketed', () => {
    const { scene, raw, log } = makeReplayScene()
    const copies = duplicateSketchSelection(
      scene,
      [{ kind: 'sketch-island', id: 40n, sketch: 3n }],
      [0.08, 0, 0],
    )
    // One gesture; the curve bracket carries the TRANSLATED center and the
    // SAME radius; both edges replay translated; the plain island 41 (not
    // selected) is untouched.
    expect(log).toEqual([
      'begin_gesture',
      'begin_curve_with(1.08,2,0,0.5)',
      'add(1.58,2,0 -> 1.08,2.5,0)',
      'add(1.08,2.5,0 -> 0.58,2,0)',
      'end_curve',
      'end_gesture',
    ])
    expect(raw.sketch_cancel_gesture).not.toHaveBeenCalled()
    // The copy comes back as the new island the replayed edges landed in.
    expect(copies).toEqual([{ kind: 'sketch-island', id: 60n, sketch: 3n }])
  })

  it('a whole-sketch selection replays every island', () => {
    const { scene, log } = makeReplayScene()
    duplicateSketchSelection(scene, [{ kind: 'sketch', id: 3n }], [0, 1, 0])
    expect(log.filter((l) => l.startsWith('add('))).toHaveLength(3)
    expect(log.filter((l) => l === 'begin_gesture')).toHaveLength(1)
    expect(log.filter((l) => l === 'end_gesture')).toHaveLength(1)
  })

  it('refuses an out-of-plane offset before anything mutates', () => {
    const { scene, raw } = makeReplayScene()
    expect(() =>
      duplicateSketchSelection(
        scene,
        [{ kind: 'sketch-island', id: 40n, sketch: 3n }],
        [0, 0, 0.5],
      ),
    ).toThrow(/PointOffPlane/)
    expect(raw.sketch_begin_gesture).not.toHaveBeenCalled()
    expect(raw.sketch_add_segment).not.toHaveBeenCalled()
  })

  it('a mid-replay failure cancels the gesture and rethrows (nothing half-copied)', () => {
    const { scene, raw } = makeReplayScene({ failOnAdd: true })
    expect(() =>
      duplicateSketchSelection(
        scene,
        [{ kind: 'sketch-island', id: 40n, sketch: 3n }],
        [0.08, 0, 0],
      ),
    ).toThrow(/WouldRetopologize/)
    expect(raw.sketch_cancel_gesture).toHaveBeenCalledTimes(1)
    expect(raw.sketch_end_gesture).not.toHaveBeenCalled()
  })
})
