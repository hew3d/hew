/**
 * transformSelection — drawn-shape routing for the transform tools.
 *
 * Pins the fixes for the "cannot rotate any shape up 90 degrees" defect
 * class on the drafting engine: a selected edge/curve transforms its UNIT
 * (island; never a silent no-op), refs group by plane context with one
 * draft per context, an out-of-plane transform detaches only a shape drawn
 * entirely over the void (every other shape refuses `NotInPlane` before
 * anything lands), and the selection is re-resolved BY GEOMETRY afterwards.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import {
  copyOwnersAndDrawing,
  draftEdgesToErase,
  commitSelectionTransform,
  duplicateSketchSelection,
  duplicateSketchSelectionByAffine,
  duplicateSketchSelectionByAffineArray,
  planDraftTransforms,
  dropRidingDrafts,
  planSketchTransforms,
  buildNodePreview,
  buildSelectionPreview,
} from './transformSelection'
import { isSessionGhost } from './transformPreview'
import { affineToFloat64, applyAffineToPoint, rotateAboutPivotAxis, translationAffine } from './transformMath'
import { ctxGround, ctxOfPlane, draftRef } from './planeUnits'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'

const AFFINE = new Float64Array([1, 0, 0, 5, 0, 1, 0, 0, 0, 0, 1, 0])
/** Straight up +Z by one metre: leaves the ground plane. */
const AFFINE_UP = new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1])

type V3 = [number, number, number]

const CTX = ctxGround()
/** The plane z = 1 (and a plane through x = 0) as canonical contexts. */
const CTX_UP = ctxOfPlane([0, 0, 1], [0, 0, 1]) as string
const CTX_X = ctxOfPlane([0, 0, 0], [1, 0, 0]) as string
const INSTANCE = 42n
const CTX_INST = ctxOfPlane([0, 0, 0], [0, 0, 1], INSTANCE) as string

/** Kernel keys are 16-hex strings. */
const hex = (n: bigint): string => n.toString(16).padStart(16, '0')
const di = (key: bigint, ctx: string = CTX): NodeRef => draftRef('draft-island', ctx, hex(key))
const de = (key: bigint, ctx: string = CTX): NodeRef => draftRef('draft-edge', ctx, hex(key))
const dc = (key: bigint, ctx: string = CTX): NodeRef => draftRef('draft-curve', ctx, hex(key))

type FakeStore =
  | { kind: 'topology'; object: number }
  | { kind: 'owned'; object: number; sketch: number }
  | { kind: 'free'; sketch: number }
interface FakeEdge {
  key: bigint
  a: V3
  b: V3
  store?: FakeStore
  curve?: { center: V3; radius: number } | null
}
/** A curve RUN of a unit: the piece of one circle (or polygon) a click selects. */
interface FakeRun {
  key: bigint
  curveKey: bigint
  kind?: 'circle' | 'polygon'
  edges: bigint[]
  center?: V3
  radius?: number
}
interface FakeUnit {
  key: bigint
  edges: FakeEdge[]
  curves?: FakeRun[]
}
const FREE3: FakeStore = { kind: 'free', sketch: 3 }
const TOPO1: FakeStore = { kind: 'topology', object: 1 }

interface FakeRegion {
  outer: V3[]
  /** The object whose face the region lies over. */
  over: number
}

function listingJson(units: readonly FakeUnit[], regions: readonly FakeRegion[] = []): string {
  return JSON.stringify({
    units: units.map((u) => ({
      key: hex(u.key),
      edges: u.edges.map((e) => ({
        key: hex(e.key),
        a: e.a,
        b: e.b,
        store: e.store ?? FREE3,
        curve: e.curve ?? null,
      })),
      curves: (u.curves ?? []).map((r) => ({
        key: hex(r.key),
        curve_key: hex(r.curveKey),
        kind: r.kind ?? 'circle',
        center: r.center ?? [0, 0, 0],
        radius: r.radius ?? 1,
        edges: r.edges.map(hex),
      })),
    })),
    regions: regions.map((r, i) => ({
      key: hex(BigInt(900 + i)),
      outer: r.outer,
      holes: [],
      over: { object: r.over, face: null },
      extrude: null,
    })),
  })
}

/** A unit whose edges are the unit-length segments of an open chain along +Y
 *  steps from (x, y0, z): edge keys given, store defaulting to free sketch 3. */
function chain(unit: bigint, edgeKeys: bigint[], at: V3 = [0, 0, 0], store: FakeStore = FREE3): FakeUnit {
  return {
    key: unit,
    edges: edgeKeys.map((k, i) => ({
      key: k,
      a: [at[0] + i, at[1], at[2]] as V3,
      b: [at[0] + i + 1, at[1], at[2]] as V3,
      store,
    })),
  }
}

interface FakeOpts {
  /** plane context -> its units. MUTABLE: tests re-assign entries (usually in
   *  `onDraftEnd` / `onTransformIslands`) to simulate what a commit lands. */
  listing?: Record<string, FakeUnit[]>
  /** plane context -> its regions (a face's footprint, for the landing order) */
  regions?: Record<string, FakeRegion[]>
  /** island ids per sketch handle (decimal string keys) */
  islands?: Record<string, bigint[]>
  /** called when a draft ends (simulate arranging against other shapes) */
  onDraftEnd?: (ctx: string) => void
  /** called inside transform_sketch_islands (simulate a detach landing) */
  onTransformIslands?: () => void
  /** `draft_transform_edges` leaves the listing untouched (nothing found at the
   *  moved positions afterwards) */
  noApply?: boolean
  /** the n-th (0-based) draft_transform_edges call throws */
  failTransformAt?: number
  /** the n-th (0-based) draft_copy_edges call throws */
  failCopyAt?: number
  /** transform_selection throws */
  failTransformSelection?: boolean
  /** the n-th (0-based) draft_add_segment call throws */
  failAddAt?: number
}

function makeScene(opts: FakeOpts = {}) {
  const calls: { name: string; args: unknown[] }[] = []
  /** Everything that touches the draft engine / listings, in call order. */
  const order: string[] = []
  const listing = (opts.listing ??= {})
  let nextDraft = 1n
  const draftCtx = new Map<bigint, string>()
  let transforms = 0
  let copies = 0
  let nextCopyKey = 1000n
  // A history bracket that snapshots the mutable drawing listing and
  // restores it on abort (the kernel restores the whole document).
  let bracket: { listing: string } | null = null
  const snapshot = (): string =>
    JSON.stringify(listing, (_k, v) => (typeof v === 'bigint' ? { big: v.toString() } : v))
  const restore = (json: string): void => {
    const back = JSON.parse(json, (_k, v) => (v !== null && typeof v === 'object' && 'big' in v ? BigInt(v.big) : v)) as Record<string, FakeUnit[]>
    for (const k of Object.keys(listing)) delete listing[k]
    Object.assign(listing, back)
  }
  /** Drawing-engine calls that redraw a shape, in order (remove / curve / segment). */
  const draws: string[] = []
  let adds = 0
  const fmt = (p: ArrayLike<number>) => Array.from(p).map((x) => +x.toFixed(6)).join(',')
  const scene = {
    draft_remove_edges: vi.fn((d: bigint, keys: BigUint64Array) => {
      order.push('remove_edges')
      draws.push(`remove:${[...keys].join(',')}`)
      const ctx = draftCtx.get(d) as string
      const set = new Set([...keys].map(hex))
      for (const u of listing[ctx] ?? []) u.edges = u.edges.filter((e) => !set.has(hex(e.key)))
      listing[ctx] = (listing[ctx] ?? []).filter((u) => u.edges.length > 0)
    }),
    draft_begin_curve: vi.fn(() => {
      draws.push('curve')
    }),
    draft_begin_curve_with: vi.fn((_d: bigint, center: Float64Array, radius: number) => {
      draws.push(`circle:${fmt(center)}:${+radius.toFixed(6)}`)
    }),
    draft_begin_polygon_with: vi.fn((_d: bigint, center: Float64Array, radius: number) => {
      draws.push(`polygon:${fmt(center)}:${+radius.toFixed(6)}`)
    }),
    draft_end_curve: vi.fn(() => {
      draws.push('end_curve')
    }),
    draft_add_segment: vi.fn((d: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
      order.push('add_segment')
      const n = adds++
      if (opts.failAddAt === n) throw new Error('WouldRetopologize: nope')
      draws.push(`seg:${fmt([ax, ay, az])}>${fmt([bx, by, bz])}`)
      const ctx = draftCtx.get(d) as string
      ;(listing[ctx] ??= []).push({ key: nextCopyKey++, edges: [{ key: nextCopyKey++, a: [ax, ay, az], b: [bx, by, bz] }] })
      return { free: () => undefined }
    }),
    history_bracket_open: vi.fn(() => {
      if (bracket !== null) throw new Error('StaleReference: a history bracket is already open')
      bracket = { listing: snapshot() }
      order.push('bracket_open')
    }),
    history_bracket_commit: vi.fn((_label: string) => {
      bracket = null
      order.push('bracket_commit')
      return true
    }),
    history_bracket_abort: vi.fn(() => {
      if (bracket === null) return
      restore(bracket.listing)
      bracket = null
      order.push('bracket_abort')
    }),
    plane_units_json: vi.fn((ctx: string) => {
      order.push('units')
      return listingJson(listing[ctx] ?? [], opts.regions?.[ctx] ?? [])
    }),
    draft_begin_ctx: vi.fn((ctx: string, label: string) => {
      const d = nextDraft++
      draftCtx.set(d, ctx)
      order.push(`begin:${label}`)
      return d
    }),
    draft_transform_edges: vi.fn((d: bigint, keys: BigUint64Array, affine: Float64Array) => {
      order.push('transform_edges')
      const n = transforms++
      if (opts.failTransformAt === n) throw new Error('EndpointNotOnBoundary: off the edge')
      if (opts.noApply) return
      const ctx = draftCtx.get(d) as string
      const set = new Set([...keys].map(hex))
      for (const u of listing[ctx] ?? []) {
        for (const e of u.edges) {
          if (!set.has(hex(e.key))) continue
          e.a = applyAffineToPoint(affine, e.a)
          e.b = applyAffineToPoint(affine, e.b)
        }
      }
    }),
    draft_copy_edges: vi.fn((d: bigint, keys: BigUint64Array, affine: Float64Array) => {
      order.push('copy_edges')
      const n = copies++
      if (opts.failCopyAt === n) throw new Error('WouldRetopologize: nope')
      const ctx = draftCtx.get(d) as string
      const set = new Set([...keys].map(hex))
      const made: FakeEdge[] = []
      for (const u of listing[ctx] ?? []) {
        for (const e of u.edges) {
          if (!set.has(hex(e.key))) continue
          made.push({
            ...e,
            key: nextCopyKey++,
            a: applyAffineToPoint(affine, e.a),
            b: applyAffineToPoint(affine, e.b),
          })
        }
      }
      ;(listing[ctx] ??= []).push({ key: nextCopyKey++, edges: made })
    }),
    draft_end: vi.fn((d: bigint) => {
      order.push('end')
      opts.onDraftEnd?.(draftCtx.get(d) as string)
      return '{"objects":[],"sketches":[],"kept_as_sketch":[]}'
    }),
    draft_cancel: vi.fn(() => {
      order.push('cancel')
    }),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])),
    // Selected groups expand to their leaves (empty here) when drawing is selected.
    group_members: vi.fn(() => []),
    sketch_island_ids: vi.fn((sketch: bigint) => opts.islands?.[String(sketch)] ?? []),
    // The island handle of the shape at these edge positions: the fake treats
    // a unit's key as its island handle.
    sketch_island_at_edge_positions: vi.fn((_s: bigint, a: Float64Array, b: Float64Array) => {
      const same = (p: V3, q: ArrayLike<number>) =>
        Math.abs(p[0] - q[0]) < 1e-9 && Math.abs(p[1] - q[1]) < 1e-9 && Math.abs(p[2] - q[2]) < 1e-9
      for (const units of Object.values(listing)) {
        for (const u of units) {
          for (const e of u.edges) {
            if ((same(e.a, a) && same(e.b, b)) || (same(e.a, b) && same(e.b, a))) return u.key
          }
        }
      }
      return undefined
    }),
    transform_sketch_islands: vi.fn((...args: unknown[]) => {
      calls.push({ name: 'transform_sketch_islands', args })
      opts.onTransformIslands?.()
    }),
    scene_undo: vi.fn(() => ({ free: () => undefined })),
    transform_selection: vi.fn((...args: unknown[]) => {
      calls.push({ name: 'transform_selection', args })
      if (opts.failTransformSelection) throw new Error('NothingToMove: refused')
    }),
    transform_def_member: vi.fn((...args: unknown[]) => {
      calls.push({ name: 'transform_def_member', args })
    }),
    transform_def_selection: vi.fn((...args: unknown[]) => {
      calls.push({ name: 'transform_def_selection', args })
    }),
  }
  return { scene: scene as unknown as WasmScene, raw: scene, calls, order, listing, draws }
}

describe('planDraftTransforms', () => {
  it('lifts an edge and a curve to their owning unit', () => {
    const circle: FakeEdge[] = [0n, 1n].map((i) => ({
      key: 20n + i,
      a: [i === 0n ? 1.5 : 1, i === 0n ? 2 : 2.5, 0] as V3,
      b: [i === 0n ? 1 : 0.5, i === 0n ? 2.5 : 2, 0] as V3,
      store: FREE3,
      curve: { center: [1, 2, 0] as V3, radius: 0.5 },
    }))
    // The circle is one curve RUN (key 98) of curve 99; a `draft-curve` keyed
    // by the run or by the circle's own curve key names every facet.
    const { scene } = makeScene({
      listing: { [CTX]: [chain(40n, [7n, 8n]), { key: 41n, edges: circle, curves: [{ key: 98n, curveKey: 99n, edges: [20n, 21n] }] }] },
    })
    const [group] = planDraftTransforms(scene, [de(7n), dc(99n)])
    expect(group.ctx).toBe(CTX)
    expect(group.unitKeys).toEqual([hex(40n), hex(41n)])
    expect(group.edges.map((e) => e.key)).toEqual([hex(7n), hex(8n), hex(20n), hex(21n)])
    // The run key lifts the same way as the circle's curve key.
    const [byRun] = planDraftTransforms(scene, [de(7n), dc(98n)])
    expect(byRun.unitKeys).toEqual([hex(40n), hex(41n)])
  })

  it('returns no group for a stale edge (pruned selection)', () => {
    const { scene } = makeScene({ listing: { [CTX]: [chain(40n, [7n])] } })
    expect(planDraftTransforms(scene, [de(9n)])).toEqual([])
  })
})

describe('planSketchTransforms', () => {
  // Drawn shapes no longer feed this planner by themselves: it plans whole
  // `'sketch'` refs plus the island groups of detaching free shapes.
  it('folds islands covering the whole sketch into a whole-sketch bake', () => {
    const { scene } = makeScene({ islands: { '3': [40n] } })
    const plan = planSketchTransforms(scene, [], [{ sketch: 3n, islands: [40n] }])
    expect(plan.sketches).toEqual([3n])
    expect(plan.islands).toEqual([])
  })

  it('keeps a strict subset of islands at island granularity', () => {
    const { scene } = makeScene({ islands: { '3': [40n, 41n] } })
    const plan = planSketchTransforms(scene, [], [{ sketch: 3n, islands: [40n] }])
    expect(plan.sketches).toEqual([])
    expect(plan.islands).toEqual([{ sketch: 3n, islands: [40n] }])
  })

  it('groups several islands of one sketch into ONE entry, one per sketch', () => {
    const { scene } = makeScene({ islands: { '3': [40n, 41n, 42n], '4': [50n, 51n] } })
    const plan = planSketchTransforms(scene, [], [
      { sketch: 3n, islands: [40n] },
      { sketch: 3n, islands: [41n] },
      { sketch: 4n, islands: [50n] },
    ])
    expect(plan.sketches).toEqual([])
    expect(plan.islands).toEqual([
      { sketch: 3n, islands: [40n, 41n] },
      { sketch: 4n, islands: [50n] },
    ])
  })

  it('dedupes an island listed twice (an edge selected alongside its own island)', () => {
    const { scene } = makeScene({ islands: { '3': [40n, 41n] } })
    const plan = planSketchTransforms(scene, [], [
      { sketch: 3n, islands: [40n] },
      { sketch: 3n, islands: [40n] },
    ])
    expect(plan.islands).toEqual([{ sketch: 3n, islands: [40n] }])
  })

  it('a whole-sketch selection absorbs its own islands', () => {
    const { scene } = makeScene({ islands: { '3': [40n, 41n] } })
    const plan = planSketchTransforms(scene, [{ kind: 'sketch', id: 3n }], [{ sketch: 3n, islands: [40n] }])
    expect(plan.sketches).toEqual([3n])
    expect(plan.islands).toEqual([])
  })
})

describe('commitSelectionTransform', () => {
  it('a selected draft-edge transforms its unit: draft_transform_edges gets ALL the unit\'s edge keys (no silent no-op)', () => {
    const { scene, raw } = makeScene({
      listing: { [CTX]: [chain(40n, [7n, 8n, 9n]), chain(41n, [10n], [0, 5, 0])] },
    })
    commitSelectionTransform(scene, [de(7n)], AFFINE)
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect(raw.draft_begin_ctx).toHaveBeenCalledWith(CTX, 'Move')
    expect(raw.draft_transform_edges).toHaveBeenCalledTimes(1)
    const [d, keys, affine] = raw.draft_transform_edges.mock.calls[0]
    expect(d).toBe(1n)
    expect([...keys]).toEqual([7n, 8n, 9n])
    expect(affine).toBe(AFFINE)
    expect(raw.draft_end).toHaveBeenCalledWith(1n)
    expect(raw.transform_selection).not.toHaveBeenCalled()
    expect(raw.transform_sketch_islands).not.toHaveBeenCalled()
  })

  it('commits the units of each context with ONE draft per context', () => {
    const { scene, raw } = makeScene({
      listing: {
        [CTX]: [chain(40n, [7n]), chain(41n, [8n, 9n], [0, 3, 0])],
        [CTX_UP]: [chain(50n, [30n], [0, 0, 1])],
      },
    })
    commitSelectionTransform(scene, [di(40n), di(41n), di(50n, CTX_UP)], AFFINE)
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(2)
    expect(raw.draft_begin_ctx).toHaveBeenNthCalledWith(1, CTX, 'Move')
    expect(raw.draft_begin_ctx).toHaveBeenNthCalledWith(2, CTX_UP, 'Move')
    expect(raw.draft_transform_edges).toHaveBeenCalledTimes(2)
    expect([...raw.draft_transform_edges.mock.calls[0][1]]).toEqual([7n, 8n, 9n])
    expect([...raw.draft_transform_edges.mock.calls[1][1]]).toEqual([30n])
    expect(raw.transform_sketch_islands).not.toHaveBeenCalled()
  })

  it('a clear landing keeps the selection untouched', () => {
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n]), chain(41n, [8n], [0, 5, 0])] } })
    const sel: NodeRef[] = [di(40n)]
    const after = commitSelectionTransform(scene, sel, AFFINE)
    expect(after).toEqual(sel)
    // Landing re-found the shape by position; no sketch island search needed.
    expect(raw.sketch_island_at_edge_positions).not.toHaveBeenCalled()
  })

  it('an arranging landing re-points the selection at the merged unit, found by geometry in a FRESH listing', () => {
    const listing: Record<string, FakeUnit[]> = {
      [CTX]: [
        {
          key: 40n,
          edges: [
            { key: 7n, a: [0, 0, 0], b: [1, 0, 0] },
            { key: 8n, a: [1, 0, 0], b: [1, 1, 0] },
          ],
        },
        chain(41n, [9n], [0, 9, 0]),
      ],
    }
    const { scene, raw, order } = makeScene({
      listing,
      // The landing merged unit 40 into one fresh unit 99 (41 is gone too).
      // Edge 7 was split by the landing (no exact match); edge 8 survives.
      onDraftEnd: (ctx) => {
        listing[ctx] = [
          {
            key: 99n,
            edges: [
              { key: 70n, a: [5, 0, 0], b: [5.5, 0, 0] },
              { key: 71n, a: [5.5, 0, 0], b: [6, 0, 0] },
              { key: 72n, a: [6, 0, 0], b: [6, 1, 0] },
            ],
          },
        ]
      },
    })
    const sel: NodeRef[] = [di(40n), { kind: 'object', id: 9n }]
    const after = commitSelectionTransform(scene, sel, AFFINE)
    expect(after).toEqual([di(99n), { kind: 'object', id: 9n }])
    // The listing was re-read AFTER the draft ended (the cache cannot serve it:
    // this fake has no history generation).
    expect(order.lastIndexOf('units')).toBeGreaterThan(order.indexOf('end'))
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
  })

  it('a selected edge and its own unit move ONCE and re-point at ONE unit ref', () => {
    const listing: Record<string, FakeUnit[]> = { [CTX]: [chain(40n, [7n]), chain(41n, [8n], [0, 5, 0])] }
    const { scene, raw } = makeScene({
      listing,
      onDraftEnd: (ctx) => {
        // The edge handle died in the re-arrange; everything is unit 99.
        listing[ctx] = [{ key: 99n, edges: [{ key: 77n, a: [5, 0, 0], b: [6, 0, 0] }] }]
      },
    })
    const after = commitSelectionTransform(scene, [de(7n), di(40n)], AFFINE)
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect([...raw.draft_transform_edges.mock.calls[0][1]]).toEqual([7n])
    expect(after).toEqual([di(99n)])
  })

  it('never returns two refs to one unit: a still-valid unit and a re-found node that merged onto it', () => {
    const listing: Record<string, FakeUnit[]> = {
      [CTX]: [chain(40n, [7n]), chain(41n, [8n], [0, 5, 0]), chain(42n, [9n], [0, 9, 0])],
    }
    const { scene } = makeScene({
      listing,
      // Both moved shapes now live in unit 41's merged shape.
      onDraftEnd: (ctx) => {
        listing[ctx] = [
          {
            key: 41n,
            edges: [
              { key: 70n, a: [5, 0, 0], b: [6, 0, 0] },
              { key: 71n, a: [5, 5, 0], b: [6, 5, 0] },
            ],
          },
        ]
      },
    })
    const after = commitSelectionTransform(scene, [di(41n), di(40n)], AFFINE)
    expect(after).toEqual([di(41n)])
  })

  it('a re-found edge and unit collapse onto one unit ref (re-resolved by geometry, not kept as the edge)', () => {
    // FLIPPED from the sketch-era expectation (`[sketch-edge 7]`): the old
    // code kept the edge ref when the landing was clear. The drawn-shape path
    // always re-resolves the moved geometry into `draft-island` refs.
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n]), chain(41n, [8n], [0, 5, 0])] } })
    const after = commitSelectionTransform(scene, [de(7n), di(40n)], AFFINE)
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect(after).toEqual([di(40n)])
  })

  it('a group that cannot be re-found after the commit is dropped; the rest of the selection stays', () => {
    const { scene } = makeScene({
      listing: { [CTX]: [chain(40n, [7n])] },
      noApply: true, // nothing is at the moved positions afterwards
    })
    const after = commitSelectionTransform(scene, [{ kind: 'object', id: 9n }, di(40n)], AFFINE)
    expect(after).toEqual([{ kind: 'object', id: 9n }])
  })

  it('only the unfindable group is dropped when two contexts commit', () => {
    const listing: Record<string, FakeUnit[]> = {
      [CTX]: [chain(40n, [7n])],
      [CTX_UP]: [chain(50n, [30n], [0, 0, 1])],
    }
    const { scene } = makeScene({
      listing,
      // The landing in the z=1 plane arranged the shape away entirely.
      onDraftEnd: (ctx) => {
        if (ctx === CTX_UP) listing[ctx] = []
      },
    })
    const after = commitSelectionTransform(scene, [di(40n), di(50n, CTX_UP)], AFFINE)
    expect(after).toEqual([di(40n)])
  })

  it('an out-of-plane move of a shape drawn entirely over the void DETACHES it and re-points at the moved unit', () => {
    const listing: Record<string, FakeUnit[]> = {
      [CTX]: [chain(40n, [7n]), chain(41n, [8n], [0, 5, 0])],
    }
    const { scene, raw, order } = makeScene({
      listing,
      islands: { '3': [40n, 41n] },
      // The island left sketch 3 for a fresh sketch: its unit now lists in the
      // plane z = 1 (the original plane carried through the affine).
      onTransformIslands: () => {
        listing[CTX] = [chain(41n, [8n], [0, 5, 0])]
        listing[CTX_UP] = [chain(60n, [70n], [0, 0, 1])]
      },
    })
    const after = commitSelectionTransform(scene, [di(40n)], AFFINE_UP)
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled() // no draft: the plane is left
    // The shape's island was found by the edge's own position, then moved.
    expect(raw.sketch_island_at_edge_positions).toHaveBeenCalledWith(
      3n, new Float64Array([0, 0, 0]), new Float64Array([1, 0, 0]),
    )
    expect(raw.transform_sketch_islands).toHaveBeenCalledWith(3n, new BigUint64Array([40n]), AFFINE_UP)
    expect(after).toEqual([di(60n, CTX_UP)])
    expect(order.length).toBeGreaterThan(0)
  })

  it('a whole-sketch-covering detach folds into the whole-sketch path (transform_selection), never the island op', () => {
    const { scene, raw } = makeScene({
      listing: { [CTX]: [chain(40n, [7n])] },
      islands: { '3': [40n] },
    })
    commitSelectionTransform(scene, [di(40n)], AFFINE_UP)
    expect(raw.transform_sketch_islands).not.toHaveBeenCalled()
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    const sketches = raw.transform_selection.mock.calls[0][2] as BigUint64Array
    expect([...sketches]).toEqual([3n])
  })

  it('refuses every group before committing any: a face shape that would leave its plane throws NotInPlane and calls nothing', () => {
    // CTX_X (plane x = 0) moved straight up Z stays in its plane and WOULD
    // draft; the ground group has a topology edge and leaves its plane. The
    // whole selection refuses up front — the in-plane draft never lands.
    const { scene, raw } = makeScene({
      listing: {
        [CTX_X]: [{ key: 50n, edges: [{ key: 30n, a: [0, 0, 0], b: [0, 1, 0], store: FREE3 }] }],
        [CTX]: [chain(40n, [7n], [0, 0, 0], TOPO1)],
      },
      islands: { '3': [40n, 41n] },
    })
    expect(() => commitSelectionTransform(scene, [di(50n, CTX_X), di(40n)], AFFINE_UP)).toThrow(/^NotInPlane: /)
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    expect(raw.draft_transform_edges).not.toHaveBeenCalled()
    expect(raw.transform_sketch_islands).not.toHaveBeenCalled()
    expect(raw.transform_selection).not.toHaveBeenCalled()
    expect(raw.scene_undo).not.toHaveBeenCalled()
  })

  it('a unit with ONE topology edge among free edges still refuses NotInPlane out of plane; all-free detaches', () => {
    const mixed: FakeUnit = {
      key: 40n,
      edges: [
        { key: 7n, a: [0, 0, 0], b: [1, 0, 0], store: FREE3 },
        { key: 8n, a: [1, 0, 0], b: [1, 1, 0], store: TOPO1 },
      ],
    }
    const { scene, raw } = makeScene({ listing: { [CTX]: [mixed] }, islands: { '3': [40n, 41n] } })
    expect(() => commitSelectionTransform(scene, [di(40n)], AFFINE_UP)).toThrow(/^NotInPlane: /)
    expect(raw.sketch_island_at_edge_positions).not.toHaveBeenCalled()
    expect(raw.transform_sketch_islands).not.toHaveBeenCalled()
    // The same shape moved IN the plane is fine for a face shape.
    commitSelectionTransform(scene, [di(40n)], AFFINE)
    expect(raw.draft_transform_edges).toHaveBeenCalledTimes(1)
  })

  it('commits objects and drawn geometry together', () => {
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n])] } })
    commitSelectionTransform(scene, [{ kind: 'object', id: 9n }, di(40n)], AFFINE)
    expect([...raw.draft_transform_edges.mock.calls[0][1]]).toEqual([7n])
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    const ids = raw.transform_selection.mock.calls[0][1] as BigUint64Array
    expect([...ids]).toEqual([9n])
  })

  it('commits objects and a DETACHING shape together (object through transform_selection, island through its own op)', () => {
    const { scene, raw } = makeScene({
      listing: { [CTX]: [chain(40n, [7n])] },
      islands: { '3': [40n, 41n] },
    })
    commitSelectionTransform(scene, [{ kind: 'object', id: 9n }, di(40n)], AFFINE_UP)
    expect(raw.transform_sketch_islands).toHaveBeenCalledWith(3n, new BigUint64Array([40n]), AFFINE_UP)
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    expect([...(raw.transform_selection.mock.calls[0][1] as BigUint64Array)]).toEqual([9n])
  })

  it("excludes drawn shapes from transform_selection's kinds/ids — only the plain object rides through it", () => {
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n])] } })
    commitSelectionTransform(scene, [{ kind: 'object', id: 3n }, di(40n)], AFFINE)
    expect(raw.transform_selection).toHaveBeenCalledWith(
      new Uint8Array([0]),
      new BigUint64Array([3n]),
      new BigUint64Array([]),
      AFFINE,
    )
    expect([...raw.draft_transform_edges.mock.calls[0][1]]).toEqual([7n])
  })

  it('skips a stale edge like any pruned handle instead of throwing', () => {
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n])] } })
    commitSelectionTransform(scene, [de(99n)], AFFINE)
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    expect(raw.transform_sketch_islands).not.toHaveBeenCalled()
    expect(raw.transform_selection).not.toHaveBeenCalled()
  })

  it('returns the selection unchanged apart from re-found shapes', () => {
    const listing: Record<string, FakeUnit[]> = {
      [CTX]: [chain(40n, [7n]), chain(50n, [5n], [0, 4, 0])],
    }
    const { scene } = makeScene({
      listing,
      // The second shape re-keys when it lands (a different unit key now).
      onDraftEnd: (ctx) => {
        listing[ctx] = [chain(40n, [7n], [5, 0, 0]), chain(77n, [6n], [5, 4, 0])]
      },
    })
    const obj: NodeRef = { kind: 'object', id: 3n }
    const committed = commitSelectionTransform(scene, [obj, di(40n), de(5n)], AFFINE)
    expect(committed).toEqual([obj, di(40n), di(77n)])
  })

  // --- retraction: a refusal after a step landed undoes exactly that step ---

  it('a LATER refusal aborts the history bracket: the landed draft is restored exactly, no scene_undo', () => {
    const { scene, raw, order } = makeScene({
      listing: {
        [CTX]: [chain(40n, [7n])],
        [CTX_UP]: [chain(50n, [30n], [0, 0, 1])],
      },
      failTransformAt: 1, // the second context's draft refuses
    })
    expect(() => commitSelectionTransform(scene, [di(40n), di(50n, CTX_UP)], AFFINE)).toThrow(
      /EndpointNotOnBoundary/,
    )
    expect(raw.draft_end).toHaveBeenCalledTimes(1) // the first draft landed…
    expect(raw.draft_cancel).toHaveBeenCalledTimes(1) // …the refusing one was cancelled…
    expect(raw.history_bracket_abort).toHaveBeenCalledTimes(1) // …and the bracket restored the landed one
    expect(raw.history_bracket_commit).not.toHaveBeenCalled()
    expect(raw.scene_undo).not.toHaveBeenCalled()
    expect(order.at(-1)).toBe('bracket_abort')
  })

  it('a refusal from transform_selection after a draft already landed aborts the bracket (draft restored)', () => {
    const { scene, raw } = makeScene({
      listing: { [CTX]: [chain(40n, [7n])] },
      failTransformSelection: true,
    })
    expect(() =>
      commitSelectionTransform(scene, [di(40n), { kind: 'object', id: 3n }], AFFINE),
    ).toThrow(/NothingToMove/)
    expect(raw.draft_end).toHaveBeenCalledTimes(1)
    expect(raw.history_bracket_abort).toHaveBeenCalledTimes(1)
    expect(raw.scene_undo).not.toHaveBeenCalled()
  })

  // component-edit-parity.md phase A2: an active instance context routes
  // every selected 'object' node through transform_def_member instead of the
  // world transform_selection — the world gesture affine passed straight
  // through, since the kernel conjugates it through the instance's pose.
  describe('with an active instance context (component-edit-parity.md phase A2)', () => {
    it('routes selected members through one atomic definition-selection call', () => {
      const { scene, raw } = makeScene()
      commitSelectionTransform(
        scene,
        [{ kind: 'object', id: 7n }, { kind: 'object', id: 8n }],
        AFFINE,
        INSTANCE,
      )
      expect(raw.transform_def_selection).toHaveBeenCalledTimes(1)
      expect([...(raw.transform_def_selection.mock.calls[0][1] as BigUint64Array)])
        .toEqual([7n, 8n])
      expect(raw.transform_selection).not.toHaveBeenCalled()
      expect(raw.transform_sketch_islands).not.toHaveBeenCalled()
    })

    it('routes a definition-owned detaching shape through the instance-aware sketch path', () => {
      const { scene, raw } = makeScene({
        listing: { [CTX_INST]: [chain(40n, [7n])] },
        islands: { '3': [40n] },
      })
      commitSelectionTransform(
        scene,
        [{ kind: 'group', id: 1n }, di(40n, CTX_INST)],
        AFFINE_UP,
        INSTANCE,
      )
      expect(raw.transform_def_member).not.toHaveBeenCalled()
      expect([...(raw.transform_def_selection.mock.calls[0][2] as BigUint64Array)])
        .toEqual([3n])
      expect(raw.transform_selection).not.toHaveBeenCalled()
      expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    })

    it('looks a free definition shape up in the DEFINITION frame (world endpoints through the inverse pose), and refuses when it is not found', () => {
      const { scene, raw } = makeScene({
        listing: { [CTX_INST]: [chain(40n, [7n], [10, 20, 0])] },
        islands: { '3': [40n, 41n] },
      })
      // The instance sits at (10, 20, 0): the listing's world (10,20,0)-(11,20,0)
      // is (0,0,0)-(1,0,0) in the definition.
      raw.instance_pose.mockReturnValue(new Float64Array([1, 0, 0, 10, 0, 1, 0, 20, 0, 0, 1, 0]))
      raw.sketch_island_at_edge_positions.mockImplementation((_s: bigint, a: Float64Array, b: Float64Array) =>
        a[0] === 0 && a[1] === 0 && b[0] === 1 && b[1] === 0 ? 40n : undefined)
      commitSelectionTransform(scene, [di(40n, CTX_INST)], AFFINE_UP, INSTANCE)
      expect([...(raw.transform_def_selection.mock.calls[0][4] as BigUint64Array)]).toEqual([40n])

      // A lookup that finds nothing is surfaced, not silently skipped.
      raw.transform_def_selection.mockClear()
      raw.sketch_island_at_edge_positions.mockReturnValue(undefined)
      expect(() => commitSelectionTransform(scene, [di(40n, CTX_INST)], AFFINE_UP, INSTANCE)).toThrow(/StaleReference/)
      expect(raw.transform_def_selection).not.toHaveBeenCalled()
    })

    it('validates and transforms a strict-subset definition sketch island', () => {
      const { scene, raw } = makeScene({
        listing: { [CTX_INST]: [chain(40n, [7n])] },
        islands: { '3': [40n, 41n] },
      })
      commitSelectionTransform(scene, [di(40n, CTX_INST)], AFFINE_UP, INSTANCE)
      expect([...(raw.transform_def_selection.mock.calls[0][3] as BigUint64Array)])
        .toEqual([3n])
      expect([...(raw.transform_def_selection.mock.calls[0][4] as BigUint64Array)])
        .toEqual([40n])
      expect(raw.transform_selection).not.toHaveBeenCalled()
    })

    it('a drawn-shape-only selection inside an instance drafts its instanced context and never calls transform_def_selection with empty arrays', () => {
      const { scene, raw } = makeScene({ listing: { [CTX_INST]: [chain(40n, [7n])] } })
      commitSelectionTransform(scene, [di(40n, CTX_INST)], AFFINE, INSTANCE)
      expect(raw.draft_begin_ctx).toHaveBeenCalledWith(CTX_INST, 'Move')
      expect([...raw.draft_transform_edges.mock.calls[0][1]]).toEqual([7n])
      expect(raw.transform_def_selection).not.toHaveBeenCalled()
    })

    it('the instance rides through in the context string, objects and drawn shapes commit side by side', () => {
      const { scene, raw } = makeScene({ listing: { [CTX_INST]: [chain(40n, [7n])] } })
      commitSelectionTransform(
        scene,
        [{ kind: 'object', id: 7n }, di(40n, CTX_INST)],
        AFFINE,
        INSTANCE,
      )
      // The draft opens on EXACTLY the shape's own context string (it names
      // the instance); no `_in_instance` begin variant exists or is needed.
      expect(raw.draft_begin_ctx.mock.calls[0][0]).toBe(CTX_INST)
      expect(JSON.parse(raw.draft_begin_ctx.mock.calls[0][0] as string).instance).toBe(Number(INSTANCE))
      expect([...(raw.transform_def_selection.mock.calls[0][1] as BigUint64Array)]).toEqual([7n])
    })

    it('a null/omitted instance falls back to the ordinary world path (unchanged)', () => {
      const { scene, raw } = makeScene()
      commitSelectionTransform(scene, [{ kind: 'object', id: 7n }], AFFINE, null)
      expect(raw.transform_def_member).not.toHaveBeenCalled()
      expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    })
  })
})

// ---------------------------------------------------------------------------
// duplicateSketchSelection — Move+Alt's sketch copy path (playtest: "you can
// Move a Sketch but you can't Copy a Sketch"). The copy is a translated
// replay through the ordinary drawing surface: one gesture per sketch (one
// undo step), curve chains re-bracketed with their translated analytic
// definition so a copied circle stays a true circle.
//
// The fake below models the kernel's REAL gesture semantics rather than just
// logging calls, because the interesting failures here are all about WHAT THE
// DOCUMENT IS LEFT HOLDING when a replay throws partway.
// ---------------------------------------------------------------------------

interface ReplayFake {
  scene: WasmScene
  raw: Record<string, ReturnType<typeof vi.fn>>
  log: string[]
  /** The live sketch content, serialized — the fake's byte-identity oracle. */
  content(): string
  /** How many steps the undo stack holds. */
  undoDepth(): number
  /** The history brackets the scene saw. */
  brackets: { opened: number; committed: string[]; aborted: number }
  /** Push an unrelated recorded action (something the user did BEFORE the
   * copy) onto the undo stack, so a stray retracting undo is detectable. */
  pushUnrelatedAction(): void
}

/**
 * A fake Scene modelling the kernel's ACTUAL sketch-gesture semantics — the
 * contract `duplicateSketchSelection` has to be correct against. Every clause
 * here is checked against `crates/kernel/src/document.rs`:
 *
 * - `sketch_add_segment` mutates the live sketch IMMEDIATELY: the kernel
 *   reaches it through `Document::sketch_mut` and each add is its own
 *   committed clone-validate-swap, never routed through the undo log. So a
 *   LATER add throwing does not un-apply the earlier ones.
 * - The gesture bracket only decides what the UNDO LOG gets:
 *   `end_sketch_gesture` diffs the sketch against the gesture's `before`
 *   snapshot, pushing exactly ONE step if it changed and nothing at all if it
 *   didn't — and closing the bracket either way, including on its Err paths.
 * - `cancel_sketch_gesture` DROPS the snapshot WITHOUT restoring it ("Any
 *   mutations made inside the abandoned bracket stay in the sketch but out of
 *   the undo log; cancel-before-mutate is the caller's contract"). It is a
 *   cancel-BEFORE-mutate primitive, not a rollback.
 * - `history_generation` moves by exactly one on every push and every undo,
 *   and on nothing else.
 *
 * Two sketches, both on the ground plane: 3 (island 40 = a 2-edge curve
 * chain, circle center (1,2,0) r 0.5; island 41 = one plain edge) and 4
 * (island 50 = one plain edge).
 *
 * `failAfterAdds: n` lets the first n `sketch_add_segment` calls land and
 * throws on the next — `n > 0` is the case a first-call-only failure test
 * structurally cannot see.
 */
function makeReplayScene(opts: { failAfterAdds?: number; failDraftCopyAt?: number; failDraftCopyCalls?: number[] } = {}): ReplayFake {
  const log: string[] = []
  let nextEdge = 500n
  let adds = 0
  // Out-of-plane copies land on fresh sketches; model each with its own id
  // and island list so `sketch_island_ids` can report the copy back.
  let nextCopySketch = 70n
  let nextCopyIsland = 80n
  const copyIslands = new Map<bigint, bigint[]>()

  // Live sketch content, keyed "<sketch>/<edge>". Seeded with the sources.
  let live = new Map<string, number[]>([
    ['3/10', [1.5, 2, 0, 1, 2.5, 0]],
    ['3/11', [1, 2.5, 0, 0.5, 2, 0]],
    ['3/20', [5, 5, 0, 6, 5, 0]],
    ['4/30', [0, 8, 0, 1, 8, 0]],
  ])
  const undoStack: { sketch: bigint; before: Map<string, number[]> }[] = []
  let pending: { sketch: bigint; before: Map<string, number[]> } | null = null
  let generation = 0n

  const ser = (m: Map<string, number[]>): string =>
    [...m.entries()]
      .sort((x, y) => (x[0] < y[0] ? -1 : x[0] > y[0] ? 1 : 0))
      .map(([k, v]) => `${k}=${v.join(',')}`)
      .join('|')
  /** Just one sketch's edges — what a gesture snapshots and diffs. */
  const ofSketch = (s: bigint): Map<string, number[]> =>
    new Map([...live].filter(([k]) => k.startsWith(`${s}/`)))

  // Island of every edge: the seeded sources, replayed edges (one fresh island
  // per source sketch), the edges of out-of-plane copies and of draft copies.
  const copyEdgeIsland = new Map<string, bigint>()
  const islandOf = (s: bigint, e: bigint): bigint | undefined => {
    const own = copyEdgeIsland.get(`${s}/${e}`)
    if (own !== undefined) return own
    if (e === 10n || e === 11n) return 40n
    if (e === 20n) return 41n
    if (e === 30n) return 50n
    if (e >= 500n) return s === 3n ? 60n : 61n
    return undefined
  }
  const liveEdges = (): { s: bigint; e: bigint; seg: number[] }[] =>
    [...live].map(([k, seg]) => {
      const [s, e] = k.split('/')
      return { s: BigInt(s), e: BigInt(e), seg }
    })
  // Draft bookkeeping (the drafting engine path for drawn-shape refs).
  const drafts = new Map<bigint, { sketch: bigint | null; before: Map<string, number[]> | null }>()
  let nextDraft = 1n
  let draftCopyCalls = 0
  let nextDraftUnit = 600n

  // History bracket: open snapshots EVERYTHING (live content, undo stack,
  // generation, bookkeeping); abort restores it exactly; commit folds every
  // entry pushed since open into ONE compound entry that undo restores whole.
  let bracket: {
    live: Map<string, number[]>
    stack: { sketch: bigint; before: Map<string, number[]> }[]
    generation: bigint
    copyEdgeIsland: Map<string, bigint>
    copyIslands: Map<bigint, bigint[]>
  } | null = null
  const brackets = { opened: 0, committed: [] as string[], aborted: 0 }

  const raw = {
    history_bracket_open: vi.fn(() => {
      if (bracket !== null) throw new Error('StaleReference: a history bracket is already open')
      brackets.opened += 1
      bracket = {
        live: new Map([...live].map(([k, v]) => [k, [...v]])),
        stack: [...undoStack],
        generation,
        copyEdgeIsland: new Map(copyEdgeIsland),
        copyIslands: new Map(copyIslands),
      }
    }),
    history_bracket_commit: vi.fn((label: string) => {
      if (bracket === null) throw new Error('StaleReference: no history bracket')
      const b = bracket
      bracket = null
      if (undoStack.length === b.stack.length) return false
      brackets.committed.push(label)
      undoStack.length = b.stack.length
      // The compound's undo restores the whole content as it was at open.
      undoStack.push({ sketch: -1n, before: b.live })
      generation = b.generation + 1n
      return true
    }),
    history_bracket_abort: vi.fn(() => {
      if (bracket === null) return
      brackets.aborted += 1
      const b = bracket
      bracket = null
      live = b.live
      undoStack.length = 0
      undoStack.push(...b.stack)
      generation = b.generation
      copyEdgeIsland.clear()
      for (const [k, v] of b.copyEdgeIsland) copyEdgeIsland.set(k, v)
      copyIslands.clear()
      for (const [k, v] of b.copyIslands) copyIslands.set(k, v)
      pending = null
    }),
    // The ground-plane listing: every live edge, grouped into units by island.
    plane_units_json: vi.fn((_ctx: string) => {
      const units = new Map<bigint, object[]>()
      for (const { s, e, seg } of liveEdges()) {
        const island = islandOf(s, e)
        if (island === undefined) continue
        const list = units.get(island) ?? []
        list.push({
          key: e.toString(16).padStart(16, '0'),
          a: seg.slice(0, 3),
          b: seg.slice(3, 6),
          store: { kind: 'free', sketch: Number(s) },
          curve: s === 3n && (e === 10n || e === 11n) ? { center: [1, 2, 0], radius: 0.5 } : null,
        })
        units.set(island, list)
      }
      return JSON.stringify({
        units: [...units].map(([k, edges]) => ({ key: k.toString(16).padStart(16, '0'), edges })),
        regions: [],
      })
    }),
    sketch_island_at_edge_positions: vi.fn((s: bigint, a: Float64Array, b: Float64Array) => {
      const near = (p: number[], q: ArrayLike<number>) =>
        Math.abs(p[0] - q[0]) < 1e-9 && Math.abs(p[1] - q[1]) < 1e-9 && Math.abs(p[2] - q[2]) < 1e-9
      for (const { s: es, e, seg } of liveEdges()) {
        if (es !== s) continue
        const p = seg.slice(0, 3)
        const q = seg.slice(3, 6)
        if ((near(p, a) && near(q, b)) || (near(p, b) && near(q, a))) return islandOf(es, e)
      }
      return undefined
    }),
    draft_begin_ctx: vi.fn((_ctx: string, label: string) => {
      log.push(`begin_draft(${label})`)
      const d = nextDraft++
      drafts.set(d, { sketch: null, before: null })
      return d
    }),
    draft_copy_edges: vi.fn((d: bigint, keys: BigUint64Array, affine: Float64Array) => {
      log.push(`copy_edges([${[...keys].join(',')}])`)
      const n = draftCopyCalls++
      if (opts.failDraftCopyCalls?.includes(n)) throw new Error('WouldRetopologize: nope')
      if (opts.failDraftCopyAt !== undefined && n >= opts.failDraftCopyAt) {
        throw new Error('WouldRetopologize: nope')
      }
      const draft = drafts.get(d)!
      const unit = nextDraftUnit++
      for (const { s, e, seg } of liveEdges()) {
        if (!keys.includes(e)) continue
        if (draft.sketch === null) {
          draft.sketch = s
          draft.before = ofSketch(s)
        }
        const id = nextEdge++
        const a = applyAffineToPoint(affine, [seg[0], seg[1], seg[2]])
        const b = applyAffineToPoint(affine, [seg[3], seg[4], seg[5]])
        copyEdgeIsland.set(`${s}/${id}`, unit)
        live.set(`${s}/${id}`, [...a, ...b])
      }
    }),
    draft_end: vi.fn((d: bigint) => {
      log.push('end_draft')
      const draft = drafts.get(d)!
      if (draft.sketch !== null && draft.before !== null) {
        undoStack.push({ sketch: draft.sketch, before: draft.before })
        generation += 1n
      }
      return '{"objects":[],"sketches":[],"kept_as_sketch":[]}'
    }),
    draft_cancel: vi.fn((d: bigint) => {
      log.push('cancel_draft')
      const draft = drafts.get(d)!
      if (draft.sketch !== null && draft.before !== null) {
        for (const k of [...live.keys()]) if (k.startsWith(`${draft.sketch}/`)) live.delete(k)
        for (const [k, v] of draft.before) live.set(k, v)
      }
    }),
    sketch_island_ids: vi.fn((s: bigint) =>
      copyIslands.get(s) ?? (s === 3n ? [40n, 41n] : s === 4n ? [50n] : []),
    ),
    sketch_island_edges: vi.fn((s: bigint, island: bigint) =>
      liveEdges()
        .filter((x) => x.s === s && islandOf(x.s, x.e) === island)
        .map((x) => x.e),
    ),
    sketch_edge_island: vi.fn((s: bigint, edge: bigint) => islandOf(s, edge)),
    sketch_edge_endpoints: vi.fn((s: bigint, e: bigint) => live.get(`${s}/${e}`)),
    sketch_edge_curve: vi.fn((s: bigint, e: bigint) =>
      s === 3n && (e === 10n || e === 11n) ? 7n : undefined,
    ),
    sketch_curve_geom: vi.fn(() => [1, 2, 0, 0.5]),
    sketch_plane: vi.fn(() => [0, 0, 0, 0, 0, 1]),
    history_generation: vi.fn(() => generation),
    sketch_begin_gesture: vi.fn((s: bigint) => {
      log.push('begin_gesture')
      pending = { sketch: s, before: ofSketch(s) }
    }),
    sketch_end_gesture: vi.fn((s: bigint) => {
      log.push('end_gesture')
      if (pending === null || pending.sketch !== s) throw new Error('SketchGestureNotOpen')
      const { before } = pending
      pending = null // the bracket closes either way
      if (ser(before) === ser(ofSketch(s))) return // unchanged: records nothing
      undoStack.push({ sketch: s, before })
      generation += 1n
    }),
    sketch_cancel_gesture: vi.fn(() => {
      log.push('cancel_gesture')
      pending = null // drops the snapshot; whatever landed STAYS live
    }),
    scene_undo: vi.fn(() => {
      log.push('scene_undo')
      const step = undoStack.pop()
      if (step === undefined) throw new Error('NothingToUndo')
      if (step.sketch === -1n) live.clear() // a bracketed gesture: restore the whole content
      else for (const k of [...live.keys()]) if (k.startsWith(`${step.sketch}/`)) live.delete(k)
      for (const [k, v] of step.before) live.set(k, v)
      generation += 1n
      return { free: () => { /* no-op */ } }
    }),
    sketch_begin_curve: vi.fn(() => { log.push('begin_curve'); return 8n }),
    sketch_begin_curve_with: vi.fn((_s: bigint, cx: number, cy: number, cz: number, r: number) => {
      log.push(`begin_curve_with(${cx},${cy},${cz},${r})`)
      return 8n
    }),
    sketch_end_curve: vi.fn(() => log.push('end_curve')),
    sketch_add_segment: vi.fn((s: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
      if (opts.failAfterAdds !== undefined && adds >= opts.failAfterAdds) {
        throw new Error('WouldRetopologize: nope')
      }
      adds += 1
      log.push(`add(${ax},${ay},${az} -> ${bx},${by},${bz})`)
      const id = nextEdge++
      live.set(`${s}/${id}`, [ax, ay, az, bx, by, bz]) // lands NOW, not at end_gesture
      return { new_edges: () => [id], free: () => { /* no-op */ } }
    }),
    // Out-of-plane copy: the kernel builds ONE NEW sketch holding ALL the
    // given islands' geometry with the affine baked in, source untouched, in
    // ONE atomic undo step. The fake mirrors that — a fresh sketch id, every
    // island's edges added lifted by the affine's Z translation, one recorded
    // step so a retraction can remove it, and nothing changed on the source.
    copy_sketch_islands: vi.fn((s: bigint, islands: BigUint64Array, affine: Float64Array) => {
      const islandList = [...islands]
      log.push(`copy_sketch_islands(${s},[${islandList.join(',')}])`)
      const copySketch = nextCopySketch++
      const before = ofSketch(copySketch) // empty — brand-new sketch
      const dz = affine[11]
      const copyIslandIds: bigint[] = []
      for (const island of islandList) {
        copyIslandIds.push(nextCopyIsland++)
        const copyIsland = copyIslandIds[copyIslandIds.length - 1]
        for (const e of raw.sketch_island_edges(s, island)) {
          const seg = live.get(`${s}/${e}`)
          if (seg === undefined) continue
          const id = nextEdge++
          copyEdgeIsland.set(`${copySketch}/${id}`, copyIsland)
          live.set(`${copySketch}/${id}`, [
            seg[0], seg[1], seg[2] + dz, seg[3], seg[4], seg[5] + dz,
          ])
        }
      }
      copyIslands.set(copySketch, copyIslandIds)
      undoStack.push({ sketch: copySketch, before }) // hide-the-copy undo
      generation += 1n
      return copySketch
    }),
  }
  return {
    scene: raw as unknown as WasmScene,
    raw,
    log,
    content: () => ser(live),
    brackets,
    undoDepth: () => undoStack.length,
    pushUnrelatedAction: () => {
      undoStack.push({ sketch: 9n, before: ofSketch(9n) })
      live.set('9/900', [0, 0, 0, 1, 0, 0])
      generation += 1n
    },
  }
}

describe('duplicateSketchSelection', () => {
  it('copies a drawn unit translated through ONE draft over ALL its edge keys (draft_copy_edges, no sketch replay)', () => {
    const { scene, raw, log } = makeReplayScene()
    const copies = duplicateSketchSelection(scene, [di(40n)], [0.08, 0, 0])
    // One draft: the unit's both edges (the curve chain 10, 11) go in together
    // with the translation affine; the plain unit 41 (not selected) is untouched.
    expect(log).toEqual(['begin_draft(Copy)', 'copy_edges([10,11])', 'end_draft'])
    expect(raw.draft_begin_ctx).toHaveBeenCalledWith(CTX, 'Copy')
    const affine = raw.draft_copy_edges.mock.calls[0][2] as Float64Array
    expect([affine[3], affine[7], affine[11]]).toEqual([0.08, 0, 0])
    expect(raw.sketch_begin_gesture).not.toHaveBeenCalled()
    expect(raw.sketch_add_segment).not.toHaveBeenCalled()
    expect(raw.scene_undo).not.toHaveBeenCalled() // a clean copy retracts nothing
    // The copy comes back as the unit the copied edges landed in (found by geometry).
    expect(copies).toEqual([di(600n)])
  })

  it('a whole-sketch replay re-brackets the curve with the TRANSLATED center and the SAME radius, inside one gesture', () => {
    const { scene, raw, log } = makeReplayScene()
    const copies = duplicateSketchSelection(scene, [{ kind: 'sketch', id: 3n }], [0.08, 0, 0])
    // One gesture; the curve bracket carries the translated center and the
    // same radius; both curve edges replay translated; the plain island 41
    // replays after the curve chains (a whole-sketch selection takes it too).
    expect(log).toEqual([
      'begin_gesture',
      'begin_curve_with(1.08,2,0,0.5)',
      'add(1.58,2,0 -> 1.08,2.5,0)',
      'add(1.08,2.5,0 -> 0.58,2,0)',
      'end_curve',
      'add(5.08,5,0 -> 6.08,5,0)',
      'end_gesture',
    ])
    expect(raw.sketch_cancel_gesture).not.toHaveBeenCalled()
    expect(raw.scene_undo).not.toHaveBeenCalled() // a clean copy retracts nothing
    // The copy comes back as the unit the replayed edges landed in.
    expect(copies).toEqual([di(60n)])
  })

  it('a whole-sketch selection replays every island', () => {
    const { scene, log } = makeReplayScene()
    duplicateSketchSelection(scene, [{ kind: 'sketch', id: 3n }], [0, 1, 0])
    expect(log.filter((l) => l.startsWith('add('))).toHaveLength(3)
    expect(log.filter((l) => l === 'begin_gesture')).toHaveLength(1)
    expect(log.filter((l) => l === 'end_gesture')).toHaveLength(1)
  })

  it('copies an out-of-plane offset onto a NEW sketch via copy_sketch_islands', () => {
    // A ground island copied straight up Z leaves its plane, so it detaches
    // onto its own new sketch (the kernel path) instead of the same-sketch
    // gesture replay — no PointOffPlane refusal any more.
    const { scene, raw } = makeReplayScene()
    const copies = duplicateSketchSelection(scene, [di(40n)], [0, 0, 0.5])
    // Neither the replay machinery nor a draft is used; the kernel copy op is
    // (the island is found by the shape's own edge positions).
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    expect(raw.sketch_island_at_edge_positions).toHaveBeenCalled()
    expect(raw.sketch_begin_gesture).not.toHaveBeenCalled()
    expect(raw.sketch_add_segment).not.toHaveBeenCalled()
    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(1)
    const [s, islands, affine] = raw.copy_sketch_islands.mock.calls[0]
    expect(s).toBe(3n)
    expect([...islands]).toEqual([40n])
    expect(affine[11]).toBeCloseTo(0.5) // the Z translation is baked in
    // The copy comes back as the island of the new sketch (id 80 here), found by geometry.
    expect(copies).toEqual([di(80n)])
  })

  it('a whole-sketch out-of-plane copy sends ALL the sketch islands in ONE call', () => {
    // Grouping matters: a region's hole boundary is its own island, so a
    // donut's islands must be copied together (one copy sketch), not one per
    // island. The whole-sketch selection therefore makes ONE copy call
    // carrying every island, and the copy comes back as that new sketch's
    // islands.
    const { scene, raw } = makeReplayScene()
    const copies = duplicateSketchSelection(scene, [{ kind: 'sketch', id: 3n }], [0, 0, 0.5])
    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(1)
    const [s, islands] = raw.copy_sketch_islands.mock.calls[0]
    expect(s).toBe(3n)
    expect([...islands]).toEqual([40n, 41n]) // both islands, together
    // Two islands on the one new sketch (70), reselected.
    expect(copies).toEqual([di(80n), di(81n)])
  })

  it('islands on DIFFERENT source sketches each get their own copy sketch', () => {
    // Grouping is per source sketch: a selection spanning two sketches makes
    // one copy call per source, each landing on its own new sketch.
    const { scene, raw } = makeReplayScene()
    const copies = duplicateSketchSelection(
      scene,
      [
        { kind: 'sketch', id: 3n },
        { kind: 'sketch', id: 4n },
      ],
      [0, 0, 0.5],
    )
    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(2)
    expect([...raw.copy_sketch_islands.mock.calls[0][1]]).toEqual([40n, 41n])
    expect([...raw.copy_sketch_islands.mock.calls[1][1]]).toEqual([50n])
    // Distinct copy sketches (70 then 71), all reselected.
    expect(copies).toEqual([di(80n), di(81n), di(82n)])
  })

  it('an out-of-plane copy is retracted whole when a LATER source-sketch copy throws', () => {
    // A selection spanning two source sketches makes one copy call each; the
    // second throws. Move+Alt is ONE user action, so the first copy must be
    // retracted too — the caller never receives a stranded copy to reselect.
    const { scene, raw, content, undoDepth } = makeReplayScene()
    // Narrowed explicitly: since Vitest 4, `getMockImplementation()` is typed
    // as a union that includes a constructor signature, so the bare result is
    // not callable. This mock is only ever a plain function.
    const realImpl = raw.copy_sketch_islands.getMockImplementation()! as (
      s: bigint,
      islands: BigUint64Array,
      affine: Float64Array,
    ) => bigint
    let calls = 0
    raw.copy_sketch_islands.mockImplementation((s: bigint, islands: BigUint64Array, a: Float64Array) => {
      calls += 1
      if (calls === 2) throw new Error('WouldRetopologize: nope')
      return realImpl(s, islands, a)
    })
    const before = content()
    expect(() =>
      duplicateSketchSelection(
        scene,
        [
          { kind: 'sketch', id: 3n },
          { kind: 'sketch', id: 4n },
        ],
        [0, 0, 0.5],
      ),
    ).toThrow(/WouldRetopologize/)
    expect(content()).toBe(before) // byte-identical: the first copy is gone too
    expect(undoDepth()).toBe(0)
  })

  // --- failure semantics: a refused copy leaves the document untouched ----
  //
  // `sketch_cancel_gesture` is NEVER the right tool here: the adds have
  // already mutated the live sketch by the time a later one throws, and
  // cancelling only drops the undo record of them. Every failure below has to
  // come back to the pre-copy content byte-for-byte.

  it('a failure on a LATER replay call leaves nothing behind (the first-call test cannot see this)', () => {
    // Island 40's 2-edge curve chain replays fine; the third add — island
    // 41's plain edge — throws. Two edges are ALREADY live in the sketch when
    // the failure arrives, which is exactly the case a fail-on-the-first-add
    // test never reaches.
    const { scene, raw, content, undoDepth } = makeReplayScene({ failAfterAdds: 2 })
    const before = content()
    expect(() =>
      duplicateSketchSelection(scene, [{ kind: 'sketch', id: 3n }], [0.08, 0, 0]),
    ).toThrow(/WouldRetopologize/)
    expect(content()).toBe(before) // byte-identical: no orphaned half-copy
    expect(undoDepth()).toBe(0) // and nothing left for Ctrl+Z to have to reach
    expect(raw.sketch_cancel_gesture).not.toHaveBeenCalled()
  })

  it('a failed sketch also retracts the copies earlier sketches already committed', () => {
    // Sketch 3 replays fully (3 adds, one recorded step); sketch 4's only add
    // throws. Move+Alt is ONE user action, so a refusal may not leave sketch
    // 3's copy stranded — the caller never receives those copies to reselect.
    const { scene, content, undoDepth } = makeReplayScene({ failAfterAdds: 3 })
    const before = content()
    expect(() =>
      duplicateSketchSelection(
        scene,
        [
          { kind: 'sketch', id: 3n },
          { kind: 'sketch', id: 4n },
        ],
        [0.08, 0, 0],
      ),
    ).toThrow(/WouldRetopologize/)
    expect(content()).toBe(before)
    expect(undoDepth()).toBe(0)
  })

  it('a first-call failure records nothing and never pops an unrelated action', () => {
    // The gesture mutated nothing, so it recorded nothing — and the
    // retraction must therefore NOT fire, or it would pop whatever the user
    // did BEFORE the copy. (The history-generation guard is what proves our
    // own step is on top; an unguarded undo here is a known-bad regression.)
    const { scene, raw, content, undoDepth, pushUnrelatedAction } = makeReplayScene({
      failAfterAdds: 0,
    })
    pushUnrelatedAction()
    const before = content()
    expect(() =>
      duplicateSketchSelection(scene, [{ kind: 'sketch', id: 3n }], [0.08, 0, 0]),
    ).toThrow(/WouldRetopologize/)
    expect(content()).toBe(before)
    expect(raw.scene_undo).not.toHaveBeenCalled()
    expect(undoDepth()).toBe(1) // the unrelated action is untouched
    // The bracket is closed with end_gesture (which records nothing for an
    // unchanged sketch), never with the non-restoring cancel.
    expect(raw.sketch_end_gesture).toHaveBeenCalledTimes(1)
    expect(raw.sketch_cancel_gesture).not.toHaveBeenCalled()
  })

  // --- the drafting path: a refused draft copy cancels its draft ---------------

  it('a first draft copy that refuses cancels its draft and never pops an unrelated action', () => {
    const { scene, raw, content, undoDepth, pushUnrelatedAction } = makeReplayScene({ failDraftCopyAt: 0 })
    pushUnrelatedAction()
    const before = content()
    expect(() => duplicateSketchSelection(scene, [di(40n)], [0.08, 0, 0])).toThrow(/WouldRetopologize/)
    expect(content()).toBe(before)
    expect(raw.draft_cancel).toHaveBeenCalledTimes(1)
    expect(raw.draft_end).not.toHaveBeenCalled()
    expect(raw.scene_undo).not.toHaveBeenCalled() // nothing landed: nothing to retract
    expect(undoDepth()).toBe(1) // the unrelated action is untouched
  })

  it('an array copy aborts its bracket, restoring the reps that already landed, when a LATER rep refuses', () => {
    // Call 2 refuses inside the single all-reps draft (cancelled whole); the
    // per-rep fallback then lands reps 1 and 2 (calls 3, 4) and refuses on
    // rep 3 (call 5): the bracket abort restores both landed reps.
    const { scene, raw, content, undoDepth } = makeReplayScene({ failDraftCopyCalls: [2, 5] })
    const before = content()
    const affines = [1, 2, 3].map((k) => affineToFloat64(translationAffine(0.5 * k, 0, 0)))
    expect(() => duplicateSketchSelectionByAffineArray(scene, [di(40n)], affines)).toThrow(/WouldRetopologize/)
    expect(raw.draft_end).toHaveBeenCalledTimes(2) // two fallback reps landed
    expect(raw.draft_cancel).toHaveBeenCalledTimes(2) // the single draft, then the third rep
    expect(raw.history_bracket_abort).toHaveBeenCalledTimes(1) // and the abort restored both landed reps
    expect(raw.scene_undo).not.toHaveBeenCalled()
    expect(content()).toBe(before)
    expect(undoDepth()).toBe(0)
  })

  it('a refusal inside the single all-reps draft cancels it; a refusing fallback leaves nothing and pops nothing', () => {
    const { scene, raw, content, undoDepth, pushUnrelatedAction } = makeReplayScene({ failDraftCopyAt: 2 })
    pushUnrelatedAction()
    const before = content()
    const affines = [1, 2, 3].map((k) => affineToFloat64(translationAffine(0.5 * k, 0, 0)))
    expect(() => duplicateSketchSelectionByAffineArray(scene, [di(40n)], affines)).toThrow(/WouldRetopologize/)
    expect(raw.draft_end).not.toHaveBeenCalled()
    expect(raw.draft_cancel).toHaveBeenCalledTimes(2) // single draft, then the first fallback rep
    expect(raw.scene_undo).not.toHaveBeenCalled() // nothing landed: nothing to retract
    expect(content()).toBe(before)
    expect(undoDepth()).toBe(1) // the unrelated action is untouched
  })

  it('a drawn unit and a whole sketch copied together are ONE undo step (was two), labeled Copy', () => {
    const { scene, raw, undoDepth, content, brackets } = makeReplayScene()
    const before = content()
    duplicateSketchSelection(scene, [di(50n), { kind: 'sketch', id: 3n }], [0.08, 0, 0])
    expect(raw.draft_end).toHaveBeenCalledTimes(1) // the draft landed…
    expect(raw.sketch_end_gesture).toHaveBeenCalledTimes(1) // …and so did the whole-sketch replay
    expect(undoDepth()).toBe(1) // folded into one entry
    expect(brackets.committed).toEqual(['Copy'])
    // One undo restores everything.
    scene.scene_undo().free()
    expect(content()).toBe(before)
    expect(undoDepth()).toBe(0)
  })

  it('a refusal from a whole-sketch replay aborts the bracket, restoring the draft copies that landed before it', () => {
    // A drawn unit and a whole sketch together: the draft copy lands first,
    // then the whole-sketch replay refuses; the draft copy must be retracted.
    const { scene, raw, content, undoDepth } = makeReplayScene({ failAfterAdds: 0 })
    const before = content()
    expect(() =>
      duplicateSketchSelection(scene, [di(50n), { kind: 'sketch', id: 3n }], [0.08, 0, 0]),
    ).toThrow(/WouldRetopologize/)
    expect(raw.draft_end).toHaveBeenCalledTimes(1)
    expect(raw.history_bracket_abort).toHaveBeenCalledTimes(1)
    expect(raw.scene_undo).not.toHaveBeenCalled()
    expect(content()).toBe(before)
    expect(undoDepth()).toBe(0)
  })
})

describe('duplicateSketchSelectionByAffine — data-driven plane routing', () => {
  /**
   * A minimal fake Scene for the Rotate+Alt drawn-copy path. One sketch
   * (id 1) on the ground plane (point origin, normal +Z) with a single
   * island (10) riding one plain edge (100) from (0,0,0) to (1,0,0) — a
   * point exactly 1 m from the origin/pivot, so an axis tilt off the true
   * normal shows up as a real, measurable displacement at that radius (the
   * "spokes/petals/bolt-ring" case this routing exists for is routinely
   * meters from its pivot, same as here).
   *
   * It also answers the drafting-engine surface (`plane_units_json` of the
   * ground plane, `draft_copy_edges`) and records every replayed / copied
   * edge so the copies can be found again by geometry.
   */
  function makeAffineScene() {
    let gen = 0n
    let nextEdge = 200n
    const added = new Map<bigint, number[]>() // replayed into sketch 1 (island 99)
    const copied = new Map<bigint, number[]>() // out-of-plane copies on sketch 900 (island 77)
    const draftUnits: { key: bigint; edges: Map<bigint, number[]> }[] = []
    let nextDraftUnit = 300n
    const unit = (key: bigint, sketch: number, edges: Map<bigint, number[]>) => ({
      key: key.toString(16).padStart(16, '0'),
      edges: [...edges].map(([k, seg]) => ({
        key: k.toString(16).padStart(16, '0'),
        a: seg.slice(0, 3),
        b: seg.slice(3, 6),
        store: { kind: 'free', sketch },
        curve: null,
      })),
    })
    const raw = {
      // Routing tests only: the bracket itself is exercised by makeReplayScene.
      history_bracket_open: vi.fn(),
      history_bracket_commit: vi.fn(() => true),
      history_bracket_abort: vi.fn(),
      plane_units_json: vi.fn((_ctx: string) =>
        JSON.stringify({
          units: [
            unit(10n, 1, new Map([[100n, [0, 0, 0, 1, 0, 0]]])),
            ...(added.size > 0 ? [unit(99n, 1, added)] : []),
            ...(copied.size > 0 ? [unit(77n, 900, copied)] : []),
            ...draftUnits.map((u) => unit(u.key, 1, u.edges)),
          ],
          regions: [],
        }),
      ),
      sketch_island_ids: vi.fn((s: bigint) => (s === 1n ? [10n] : s === 900n ? [77n] : [])),
      sketch_edge_island: vi.fn((_s: bigint, e: bigint) => (e >= 200n ? 99n : e === 100n ? 10n : undefined)),
      sketch_plane: vi.fn((s: bigint) =>
        s === 1n || s === 900n ? new Float64Array([0, 0, 0, 0, 0, 1]) : undefined,
      ),
      sketch_island_edges: vi.fn((_s: bigint, island: bigint) =>
        island === 10n ? [100n] : island === 99n ? [...added.keys()] : island === 77n ? [...copied.keys()] : [],
      ),
      sketch_edge_endpoints: vi.fn((_s: bigint, e: bigint) =>
        e === 100n ? [0, 0, 0, 1, 0, 0] : added.get(e) ?? copied.get(e),
      ),
      sketch_island_at_edge_positions: vi.fn((s: bigint, a: Float64Array, b: Float64Array) => {
        const same = (p: number[], q: Float64Array) => p.every((v, i) => Math.abs(v - q[i]) < 1e-9)
        const ends = [[0, 0, 0], [1, 0, 0]]
        return s === 1n && ((same(ends[0], a) && same(ends[1], b)) || (same(ends[0], b) && same(ends[1], a)))
          ? 10n
          : undefined
      }),
      sketch_edge_curve: vi.fn(() => undefined),
      sketch_begin_gesture: vi.fn(),
      sketch_end_gesture: vi.fn(() => { gen += 1n }),
      sketch_begin_curve: vi.fn(() => 1n),
      sketch_begin_curve_with: vi.fn(() => 1n),
      sketch_end_curve: vi.fn(),
      sketch_add_segment: vi.fn(
        (_s: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
          const id = nextEdge++
          added.set(id, [ax, ay, az, bx, by, bz])
          return { new_edges: () => [id], free: () => { /* no-op */ } }
        },
      ),
      history_generation: vi.fn(() => gen),
      scene_undo: vi.fn(() => { gen += 1n; return { free: () => { /* no-op */ } } }),
      copy_sketch_islands: vi.fn((_s: bigint, _islands: BigUint64Array, affine: Float64Array) => {
        const a = applyAffineToPoint(affine, [0, 0, 0])
        const b = applyAffineToPoint(affine, [1, 0, 0])
        copied.set(nextEdge++, [...a, ...b])
        gen += 1n
        return 900n
      }),
      draft_begin_ctx: vi.fn((_ctx: string, _label: string) => 1n),
      draft_copy_edges: vi.fn((_d: bigint, keys: BigUint64Array, affine: Float64Array) => {
        if (!keys.includes(100n)) return
        const a = applyAffineToPoint(affine, [0, 0, 0])
        const b = applyAffineToPoint(affine, [1, 0, 0])
        draftUnits.push({ key: nextDraftUnit++, edges: new Map([[nextEdge++, [...a, ...b]]]) })
      }),
      draft_end: vi.fn(() => {
        gen += 1n
        return '{"objects":[],"sketches":[],"kept_as_sketch":[]}'
      }),
      draft_cancel: vi.fn(),
    }
    return { scene: raw as unknown as WasmScene, raw }
  }

  /** The drawn unit (what a pointer pick selects) and the whole sketch. */
  const SELECTION: NodeRef[] = [di(10n)]
  const SKETCH_SELECTION: NodeRef[] = [{ kind: 'sketch', id: 1n }]
  const tiltedAffine = (tilt: number, angle: number) =>
    affineToFloat64(rotateAboutPivotAxis(0, 0, 0, Math.sin(tilt), 0, Math.cos(tilt), angle))

  it('an axis tilted ~1e-5 rad off the plane normal — inside the OLD cosine tolerance — routes a whole sketch to the NEW-SKETCH arm once real geometry ~1 m from the axis is measured', () => {
    // This tilt is well inside the prior analytic classifier's cosine gate
    // (dot(transformedNormal, normal) > 1 - 1e-9, i.e. angles up to ~4.5e-5
    // rad): the prior code would have classified this as in-plane and
    // replayed via sketch_add_segment, which the real kernel's PLANE_DIST
    // (1e-9 m) would then throw on for a point this far from the axis. The
    // data-driven check must catch what the analytic one could not.
    const { scene, raw } = makeAffineScene()
    const affine = tiltedAffine(1e-5, Math.PI / 2)

    const result = duplicateSketchSelectionByAffine(scene, SKETCH_SELECTION, affine)

    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(1)
    expect(raw.sketch_add_segment).not.toHaveBeenCalled() // never a same-sketch replay
    expect(result).toEqual([di(77n)])
  })

  it('an exactly-normal axis still replays a whole sketch into the SAME sketch, with the fed point already on the plane', () => {
    const { scene, raw } = makeAffineScene()
    const affine = affineToFloat64(rotateAboutPivotAxis(0, 0, 0, 0, 0, 1, Math.PI / 2))

    const result = duplicateSketchSelectionByAffine(scene, SKETCH_SELECTION, affine)

    expect(raw.sketch_add_segment).toHaveBeenCalledTimes(1)
    expect(raw.copy_sketch_islands).not.toHaveBeenCalled()
    const [sketch, ax, ay, az, bx, by, bz] = raw.sketch_add_segment.mock.calls[0]
    expect(sketch).toBe(1n)
    expect(ax).toBeCloseTo(0, 9)
    expect(ay).toBeCloseTo(0, 9)
    expect(az).toBe(0)
    expect(bx).toBeCloseTo(0, 9)
    expect(by).toBeCloseTo(1, 9)
    expect(bz).toBe(0)
    expect(result).toEqual([di(99n)])
  })

  it('a within-margin tilt still replays a whole sketch same-sketch, with the fed point projected EXACTLY onto the plane (not merely close)', () => {
    // sin(1e-10) ≈ 1e-10 m of RAW displacement at r=1 m — comfortably
    // inside the 0.5e-9 m data-driven margin, so this routes same-sketch;
    // the assertion below proves the fed point is the exact projection
    // (bit-exact zero plane distance), not that small raw residual.
    const { scene, raw } = makeAffineScene()
    const affine = tiltedAffine(1e-10, Math.PI / 2)

    duplicateSketchSelectionByAffine(scene, SKETCH_SELECTION, affine)

    expect(raw.copy_sketch_islands).not.toHaveBeenCalled()
    expect(raw.sketch_add_segment).toHaveBeenCalledTimes(1)
    const [, , , , , , bz] = raw.sketch_add_segment.mock.calls[0]
    expect(bz).toBe(0) // exact projection, not the ~1e-10 raw offset
  })

  // The same three cases for a DRAWN UNIT (a `draft-island` ref). The in-plane
  // arm is now a draft copy (`draft_copy_edges`): the kernel's draft owns the
  // plane snapping, so the affine is handed over verbatim rather than
  // projected here; the out-of-plane arm is the all-free detach copy.
  it('a drawn unit under a ~1e-5 tilt (off the plane) detaches onto a NEW sketch, never a draft or a replay', () => {
    const { scene, raw } = makeAffineScene()
    const affine = tiltedAffine(1e-5, Math.PI / 2)

    const result = duplicateSketchSelectionByAffine(scene, SELECTION, affine)

    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(1)
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    expect(raw.draft_copy_edges).not.toHaveBeenCalled()
    expect(raw.sketch_add_segment).not.toHaveBeenCalled()
    expect(raw.sketch_island_at_edge_positions).toHaveBeenCalledWith(
      1n, new Float64Array([0, 0, 0]), new Float64Array([1, 0, 0]),
    )
    expect(result).toEqual([di(77n)])
  })

  it('a drawn unit rotated about the exact plane normal copies through ONE draft over its edge keys', () => {
    const { scene, raw } = makeAffineScene()
    const affine = affineToFloat64(rotateAboutPivotAxis(0, 0, 0, 0, 0, 1, Math.PI / 2))

    const result = duplicateSketchSelectionByAffine(scene, SELECTION, affine)

    expect(raw.copy_sketch_islands).not.toHaveBeenCalled()
    expect(raw.sketch_add_segment).not.toHaveBeenCalled()
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect(raw.draft_begin_ctx).toHaveBeenCalledWith(CTX, 'Copy')
    expect(raw.draft_copy_edges).toHaveBeenCalledTimes(1)
    const [, keys, passed] = raw.draft_copy_edges.mock.calls[0]
    expect([...keys]).toEqual([100n])
    expect(passed).toBe(affine) // verbatim: no JS-side projection any more
    expect(raw.draft_end).toHaveBeenCalledTimes(1)
    expect(result).toEqual([di(300n)])
  })

  it('a drawn unit under a within-margin tilt (1e-10) still copies through a draft (in the plane), affine verbatim', () => {
    const { scene, raw } = makeAffineScene()
    const affine = tiltedAffine(1e-10, Math.PI / 2)

    duplicateSketchSelectionByAffine(scene, SELECTION, affine)

    expect(raw.copy_sketch_islands).not.toHaveBeenCalled()
    expect(raw.draft_copy_edges).toHaveBeenCalledTimes(1)
    expect(raw.draft_copy_edges.mock.calls[0][2]).toBe(affine)
  })

  // Rotate+Alt's ×N / ÷N array refinement applied to sketch sources
  // (tool-parity playtest fix): `duplicateSketchSelectionByAffineArray`
  // generalizes the single-affine function above to an ORDERED array of
  // cumulative affines, one per copy. Same fixture (`makeAffineScene`) as the
  // single-affine tests above.
  it('an in-plane N-rep array replays a whole sketch into the SAME sketch inside ONE gesture bracket (edge count = N, ONE undo restores the whole array)', () => {
    const { scene, raw } = makeAffineScene()
    // 5 cumulative 90° reps about the source plane's own normal (+Z through
    // the origin) — always in-plane, whatever the multiple.
    const affines = [1, 2, 3, 4, 5].map((k) =>
      affineToFloat64(rotateAboutPivotAxis(0, 0, 0, 0, 0, 1, k * (Math.PI / 2))),
    )

    const result = duplicateSketchSelectionByAffineArray(scene, SKETCH_SELECTION, affines)

    expect(raw.copy_sketch_islands).not.toHaveBeenCalled()
    // ONE edge per rep (island 10 rides a single plain edge) — 5 total.
    expect(raw.sketch_add_segment).toHaveBeenCalledTimes(5)
    // Every add lands on the SOURCE sketch, never a new one.
    for (const call of raw.sketch_add_segment.mock.calls) expect(call[0]).toBe(1n)
    // ONE bracket for all 5 reps — the kernel invariant that makes this ONE
    // undo step regardless of N (verified at the kernel level; this is the
    // JS-side guarantee that funnels all N reps through it).
    expect(raw.sketch_begin_gesture).toHaveBeenCalledTimes(1)
    expect(raw.sketch_end_gesture).toHaveBeenCalledTimes(1)

    // The 5 reps land at 90°, 180°, 270°, 360°(=0°), 450°(=90°) — the first
    // rep's far endpoint (1,0,0) rotated 90° about Z lands at (0,1,0).
    const [, , , , bx0, by0] = raw.sketch_add_segment.mock.calls[0]
    expect(bx0).toBeCloseTo(0, 9)
    expect(by0).toBeCloseTo(1, 9)
    // The second rep (180°) lands the far endpoint at (-1,0,0).
    const [, , , , bx1, by1] = raw.sketch_add_segment.mock.calls[1]
    expect(bx1).toBeCloseTo(-1, 9)
    expect(by1).toBeCloseTo(0, 9)

    expect(result.length).toBeGreaterThan(0)
  })

  it('an in-plane N-rep array of a DRAWN unit costs ONE draft (one undo step) with a copy per rep, each with its OWN cumulative affine', () => {
    const { scene, raw } = makeAffineScene()
    const affines = [1, 2, 3, 4, 5].map((k) =>
      affineToFloat64(rotateAboutPivotAxis(0, 0, 0, 0, 0, 1, k * (Math.PI / 2))),
    )

    const result = duplicateSketchSelectionByAffineArray(scene, SELECTION, affines)

    expect(raw.copy_sketch_islands).not.toHaveBeenCalled()
    expect(raw.sketch_add_segment).not.toHaveBeenCalled()
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect(raw.draft_end).toHaveBeenCalledTimes(1)
    expect(raw.draft_cancel).not.toHaveBeenCalled()
    expect(raw.draft_copy_edges).toHaveBeenCalledTimes(5)
    raw.draft_copy_edges.mock.calls.forEach((call, i) => expect(call[2]).toBe(affines[i]))
    // Every rep copies the ORIGINAL's keys inside the one draft.
    for (const call of raw.draft_copy_edges.mock.calls) expect([...call[1]]).toEqual([100n])
    expect(result.length).toBeGreaterThan(0)
  })

  it('an out-of-plane N-rep array costs one copy_sketch_islands call PER rep (no batching primitive exists), each with its OWN cumulative affine', () => {
    const { scene, raw } = makeAffineScene()
    // An axis tilted well outside the data-driven margin (matches the
    // existing single-affine "routes to the new-sketch arm" case above),
    // applied at 3 cumulative steps.
    const affines = [1, 2, 3].map((k) => tiltedAffine(1e-5, k * (Math.PI / 2)))

    const result = duplicateSketchSelectionByAffineArray(scene, SKETCH_SELECTION, affines)

    expect(raw.sketch_add_segment).not.toHaveBeenCalled() // never a same-sketch replay
    // One call PER rep — no kernel primitive batches these (unlike the
    // in-plane bracket above), so 3 reps cost 3 atomic copies.
    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(3)
    // Each call carries a DIFFERENT (cumulative) affine, not the same one 3×.
    const passedAffines = raw.copy_sketch_islands.mock.calls.map((c: unknown[]) => c[2])
    expect(passedAffines[0]).not.toEqual(passedAffines[1])
    expect(passedAffines[1]).not.toEqual(passedAffines[2])
    // This fixture's `copy_sketch_islands` always returns the same fixed
    // sketch id (900n, unlike RotateTool.test.ts's incrementing version) —
    // so all 3 reps report the same new unit; the call count above is what
    // proves all 3 reps actually ran.
    expect(result).toEqual([di(77n), di(77n), di(77n)])
  })

  it('an out-of-plane N-rep array of a DRAWN unit detaches per rep (the same primitive), never drafting', () => {
    const { scene, raw } = makeAffineScene()
    const affines = [1, 2, 3].map((k) => tiltedAffine(1e-5, k * (Math.PI / 2)))

    const result = duplicateSketchSelectionByAffineArray(scene, SELECTION, affines)

    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(3)
    expect(result).toEqual([di(77n), di(77n), di(77n)])
  })
})

describe('duplicateSketchSelection — drawn shapes through draft_copy_edges', () => {
  it('copies a unit with ONE draft per context over ALL its edge keys, and hands back the copy as a unit ref', () => {
    const { scene, raw } = makeScene({
      listing: {
        [CTX]: [chain(40n, [7n, 8n]), chain(41n, [9n], [0, 5, 0])],
        [CTX_UP]: [chain(50n, [30n], [0, 0, 1])],
      },
    })
    const copies = duplicateSketchSelection(scene, [de(7n), di(40n), di(50n, CTX_UP)], [3, 0, 0])
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(2)
    expect(raw.draft_begin_ctx).toHaveBeenNthCalledWith(1, CTX, 'Copy')
    expect(raw.draft_begin_ctx).toHaveBeenNthCalledWith(2, CTX_UP, 'Copy')
    expect([...raw.draft_copy_edges.mock.calls[0][1]]).toEqual([7n, 8n]) // edge + unit: once
    expect([...raw.draft_copy_edges.mock.calls[1][1]]).toEqual([30n])
    expect(raw.draft_transform_edges).not.toHaveBeenCalled() // the originals stay
    // Copies are the fresh units (the fake mints edge keys then the unit key).
    expect(copies).toEqual([di(1002n), di(1004n, CTX_UP)])
  })

  it('an array of affines gives ONE draft per context: N draft_copy_edges over the original keys, one undo step', () => {
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n, 8n])] } })
    const affines = [1, 2, 3].map((k) => affineToFloat64(translationAffine(0, 3 * k, 0)))
    const copies = duplicateSketchSelectionByAffineArray(scene, [di(40n)], affines)
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(1)
    expect(raw.draft_end).toHaveBeenCalledTimes(1)
    expect(raw.draft_cancel).not.toHaveBeenCalled()
    expect(raw.draft_copy_edges).toHaveBeenCalledTimes(3)
    raw.draft_copy_edges.mock.calls.forEach((call, i) => {
      expect(call[0]).toBe(1n) // the same draft handle every rep
      expect(call[2]).toBe(affines[i])
      expect([...call[1]]).toEqual([7n, 8n])
    })
    expect(copies).toHaveLength(3)
    expect(copies.every((c) => c.kind === 'draft-island' && c.ctx === CTX)).toBe(true)
  })

  it('two contexts in one array copy are two drafts (one per ctx), each holding every rep', () => {
    const { scene, raw } = makeScene({
      listing: { [CTX]: [chain(40n, [7n, 8n])], [CTX_UP]: [chain(50n, [30n], [0, 0, 1])] },
    })
    const affines = [1, 2].map((k) => affineToFloat64(translationAffine(2 * k, 0, 0)))
    duplicateSketchSelectionByAffineArray(scene, [di(40n), di(50n, CTX_UP)], affines)
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(2)
    expect(raw.draft_end).toHaveBeenCalledTimes(2)
    expect(raw.draft_copy_edges).toHaveBeenCalledTimes(4)
  })

  it('when the single draft throws with several reps, it is cancelled and each rep gets its own draft, re-reading the originals by geometry', () => {
    const listing: Record<string, FakeUnit[]> = { [CTX]: [chain(40n, [7n, 8n])] }
    let reps = 0
    const { scene, raw } = makeScene({
      listing,
      // The call-1 refusal is inside the single draft (rep 2 of 3).
      failCopyAt: 1,
      // The first fallback rep's landing arranges against the ORIGINAL and re-keys it.
      onDraftEnd: (ctx) => {
        reps += 1
        if (reps === 1) {
          const original = listing[ctx].find((u) => u.key === 40n)!
          original.edges = original.edges.map((e, i) => ({ ...e, key: BigInt(70 + i) }))
        }
      },
    })
    const affines = [1, 2, 3].map((k) => affineToFloat64(translationAffine(0, 3 * k, 0)))
    const copies = duplicateSketchSelectionByAffineArray(scene, [di(40n)], affines)
    expect(raw.draft_begin_ctx).toHaveBeenCalledTimes(4) // the single draft + one per rep
    expect(raw.draft_cancel).toHaveBeenCalledTimes(1)
    expect(raw.draft_end).toHaveBeenCalledTimes(3)
    // calls 0,1 = the cancelled single draft; calls 2..4 = the fallback reps.
    const calls = raw.draft_copy_edges.mock.calls
    expect(calls).toHaveLength(5)
    expect([...calls[2][1]]).toEqual([7n, 8n])
    expect([...calls[3][1]]).toEqual([70n, 71n]) // re-keyed original
    expect([...calls[4][1]]).toEqual([70n, 71n])
    for (let i = 0; i < 3; i++) expect(calls[2 + i][2]).toBe(affines[i])
    expect(copies.length).toBeGreaterThanOrEqual(3)
    expect(copies.every((c) => c.kind === 'draft-island' && c.ctx === CTX)).toBe(true)
  })

  it('a unit with a topology edge refuses NotInPlane for an out-of-plane copy, before anything lands', () => {
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n], [0, 0, 0], TOPO1)] } })
    expect(() => duplicateSketchSelection(scene, [di(40n)], [0, 0, 1])).toThrow(/^NotInPlane: /)
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    expect(raw.scene_undo).not.toHaveBeenCalled()
  })

  it('a unit with a topology edge still copies IN the plane (a face shape stays on its face)', () => {
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n], [0, 0, 0], TOPO1)] } })
    duplicateSketchSelection(scene, [di(40n)], [2, 0, 0])
    expect(raw.draft_copy_edges).toHaveBeenCalledTimes(1)
  })

  it('a rotation array that leaves the plane for ANY rep refuses the whole array for a face shape', () => {
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n], [0, 0, 0], TOPO1)] } })
    const affines = [affineToFloat64(translationAffine(1, 0, 0)), affineToFloat64(translationAffine(1, 0, 1))]
    expect(() => duplicateSketchSelectionByAffineArray(scene, [di(40n)], affines)).toThrow(/^NotInPlane: /)
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled() // decided once, up front
  })

  it('an instanced context passes through to the draft verbatim', () => {
    const { scene, raw } = makeScene({ listing: { [CTX_INST]: [chain(40n, [7n])] } })
    duplicateSketchSelection(scene, [di(40n, CTX_INST)], [1, 0, 0])
    expect(raw.draft_begin_ctx).toHaveBeenCalledWith(CTX_INST, 'Copy')
  })
})

describe('buildNodePreview — drawn-shape ghost', () => {
  function sceneWithListing(units: FakeUnit[]): WasmScene {
    return { plane_units_json: () => listingJson(units) } as unknown as WasmScene
  }

  it("returns a LineSegments ghost built from a drawn unit's line work", () => {
    const preview = buildNodePreview(
      sceneWithListing([chain(40n, [7n, 8n, 9n])]),
      null,
      null,
      di(40n),
    )
    expect(preview).toBeInstanceOf(THREE.LineSegments)
    const pos = (preview as THREE.LineSegments).geometry.getAttribute('position')
    expect(pos.count).toBe(6) // three edges, two vertices each
  })

  it('previews the WHOLE unit for an edge ref, whatever store the edge lives in (a face shape too)', () => {
    const preview = buildNodePreview(
      sceneWithListing([chain(40n, [7n, 8n], [0, 0, 0], TOPO1)]),
      null,
      null,
      de(8n),
    )
    expect(preview).toBeInstanceOf(THREE.LineSegments)
    expect((preview as THREE.LineSegments).geometry.getAttribute('position').count).toBe(4)
  })

  it('applies no instance pose to a drawn ghost (plane units are WORLD)', () => {
    const poseSpy = vi.fn(() => undefined)
    const scene = {
      plane_units_json: () => listingJson([chain(40n, [7n])]),
      instance_pose: poseSpy,
    } as unknown as WasmScene
    const preview = buildNodePreview(scene, null, null, di(40n, CTX_INST), INSTANCE)
    expect(preview).toBeInstanceOf(THREE.LineSegments)
    expect(poseSpy).not.toHaveBeenCalled()
  })

  it('returns null for a stale drawn ref — nothing to preview', () => {
    const preview = buildNodePreview(sceneWithListing([]), null, null, di(999n))
    expect(preview).toBeNull()
  })
})

describe('buildNodePreview / buildSelectionPreview — the ghost root the tool drives is marked as session geometry', () => {
  function sceneWithListing(units: FakeUnit[]): WasmScene {
    return { plane_units_json: () => listingJson(units) } as unknown as WasmScene
  }
  function objectsGroupWith(id: bigint): THREE.Group {
    const objectsGroup = new THREE.Group()
    const sub = new THREE.Group()
    sub.name = `Object_${id}`
    sub.add(new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial()))
    objectsGroup.add(sub)
    return objectsGroup
  }

  it('a single drawn shape: the LineSegments ghost itself', () => {
    const preview = buildNodePreview(sceneWithListing([chain(40n, [7n, 8n])]), null, null, di(40n))
    expect(preview).not.toBeNull()
    expect(isSessionGhost(preview!)).toBe(true)
  })

  it('a single world object: the faded clone', () => {
    const preview = buildNodePreview({} as WasmScene, objectsGroupWith(5n), null, { kind: 'object', id: 5n })
    expect(preview).not.toBeNull()
    expect(isSessionGhost(preview!)).toBe(true)
  })

  it('a multi-node selection: the shared container the tool drives, not just its children', () => {
    const scene = sceneWithListing([chain(40n, [7n, 8n])])
    const preview = buildSelectionPreview(scene, objectsGroupWith(5n), null, [{ kind: 'object', id: 5n }, di(40n)])
    expect(preview).not.toBeNull()
    expect(preview!.children).toHaveLength(2)
    expect(isSessionGhost(preview!)).toBe(true)
  })

  it('a definition sketch through an active instance: the posed wrapper the tool drives', () => {
    const scene = {
      sketch_lines: () => new Float32Array([0, 0, 0, 1, 0, 0]),
      instance_pose: () => new Float64Array([1, 0, 0, 2, 0, 1, 0, 0, 0, 0, 1, 0]),
    } as unknown as WasmScene
    const preview = buildNodePreview(scene, null, null, { kind: 'sketch', id: 3n }, 77n)
    expect(preview).not.toBeNull()
    expect(preview).toBeInstanceOf(THREE.Group)
    expect(isSessionGhost(preview!)).toBe(true)
  })
})

describe('drawn shapes ride with their own selected object', () => {
  const TOPO_UNIT = (): FakeUnit => chain(60n, [30n, 31n], [0, 0, 1], TOPO1)
  const listing = () => ({ [CTX_UP]: [TOPO_UNIT()], [CTX]: [chain(40n, [7n])] })
  const box: NodeRef = { kind: 'object', id: 1n }

  it('plans no draft group for a face shape whose object is also selected', () => {
    const { scene } = makeScene({ listing: listing() })
    expect(planDraftTransforms(scene, [box, di(60n, CTX_UP)])).toEqual([])
    // Without the object, the face shape still moves as a draft.
    expect(planDraftTransforms(scene, [di(60n, CTX_UP)])).toHaveLength(1)
  })

  it('never drops ground (free) drawing, nor a face shape of an unselected object', () => {
    const { scene } = makeScene({ listing: listing() })
    expect(dropRidingDrafts(scene, [box, di(40n)])).toEqual([box, di(40n)])
    expect(dropRidingDrafts(scene, [{ kind: 'object', id: 2n }, di(60n, CTX_UP)])).toHaveLength(2)
  })

  it('a vertical move of box + face shape succeeds (no NotInPlane) and drafts nothing', () => {
    const { scene, raw } = makeScene({ listing: listing() })
    expect(() => commitSelectionTransform(scene, [box, di(60n, CTX_UP)], AFFINE_UP)).not.toThrow()
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
  })

  it('an in-plane move of box + face shape transforms the object only, never the shape as a draft', () => {
    const { scene, raw } = makeScene({ listing: listing() })
    commitSelectionTransform(scene, [box, di(60n, CTX_UP)], AFFINE)
    expect(raw.draft_transform_edges).not.toHaveBeenCalled()
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
  })

  it('drops shapes inside a selected instance, by the context instance', () => {
    const { scene } = makeScene({ listing: { [CTX_INST]: [chain(61n, [32n], [0, 0, 0], TOPO1)] } })
    const inst: NodeRef = { kind: 'instance', id: INSTANCE }
    expect(dropRidingDrafts(scene, [inst, di(61n, CTX_INST)])).toEqual([inst])
  })
})

describe('a MIXED shape (part on a selected object\'s face, part over the void)', () => {
  const box: NodeRef = { kind: 'object', id: 1n }
  /** One island in the top plane: two edges on the box's top face (key 30,
   *  31) and two free edges hanging past it (32, 33). */
  const mixedUnit = (): FakeUnit => ({
    key: 60n,
    edges: [
      { key: 30n, a: [0, 0, 1], b: [1, 0, 1], store: TOPO1 },
      { key: 31n, a: [1, 0, 1], b: [1, 1, 1], store: TOPO1 },
      { key: 32n, a: [1, 1, 1], b: [2, 1, 1], store: FREE3 },
      { key: 33n, a: [2, 1, 1], b: [2, 0, 1], store: FREE3 },
    ],
  })
  const listing = () => ({ [CTX_UP]: [mixedUnit()] })
  const keysOf = (raw: ReturnType<typeof makeScene>['raw'], call: number): string[] =>
    [...(raw.draft_transform_edges.mock.calls[call][1] as BigUint64Array)].map(hex)

  it('keeps the shape in the selection but plans only its non-riding edges, flagged mixed', () => {
    const { scene } = makeScene({ listing: listing() })
    expect(dropRidingDrafts(scene, [box, di(60n, CTX_UP)])).toEqual([box, di(60n, CTX_UP)])
    const [g] = planDraftTransforms(scene, [box, di(60n, CTX_UP)])
    expect(g.mixed).toBe(true)
    expect(g.edges.map((e) => e.key)).toEqual([hex(32n), hex(33n)])
    // Without the object everything is drafted and nothing is mixed.
    const [all] = planDraftTransforms(scene, [di(60n, CTX_UP)])
    expect(all.mixed).toBe(false)
    expect(all.edges).toHaveLength(4)
  })

  const segs = (draws: string[]) => draws.filter((d) => d.startsWith('seg:'))

  it('an in-plane move draws it there: erases ONLY the free edges, moves the object, then draws them again mapped', () => {
    const { scene, raw, order, draws } = makeScene({ listing: listing() })
    commitSelectionTransform(scene, [box, di(60n, CTX_UP)], AFFINE)
    // (a) erase the non-riding edges; no key-moving draft for a mixed shape
    expect(draws[0]).toBe(`remove:${[32n, 33n].join(',')}`)
    expect(raw.draft_transform_edges).not.toHaveBeenCalled()
    // (b) the object moves; (c) the captured edges are drawn again, +5 in x
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    expect(segs(draws)).toEqual(['seg:6,1,1>7,1,1', 'seg:7,1,1>7,0,1'])
    expect(order.indexOf('remove_edges')).toBeLessThan(order.indexOf('add_segment'))
    expect(raw.transform_selection.mock.invocationCallOrder[0]).toBeGreaterThan(
      raw.draft_remove_edges.mock.invocationCallOrder[0],
    )
    expect(raw.transform_selection.mock.invocationCallOrder[0]).toBeLessThan(
      raw.draft_add_segment.mock.invocationCallOrder[0],
    )
    // One gesture, one history step, labeled for the tool.
    expect(raw.history_bracket_open).toHaveBeenCalledTimes(1)
    expect(raw.history_bracket_commit).toHaveBeenCalledWith('Move')
    expect(raw.history_bracket_abort).not.toHaveBeenCalled()
  })

  describe('the order never depends on where the shape lands (no footprint heuristic)', () => {
    /** The box's top face, a unit square under the face part (x 0..1). */
    const footprint = (): Record<string, FakeRegion[]> => ({
      [CTX_UP]: [{ outer: [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]], over: 1 }],
    })
    const west = affineToFloat64(translationAffine(-1.5, 0, 0))
    const wide = (): Record<string, FakeRegion[]> => ({
      [CTX_UP]: [{ outer: [[-3, -1, 1], [3, -1, 1], [3, 2, 1], [-3, 2, 1]] as V3[], over: 1 }],
    })

    it.each([
      ['east, clear of the old face', AFFINE, footprint],
      ['west, onto the old face', west, footprint],
      ['west, under a wide moved face', west, wide],
    ])('erase, then the object, then redraw — %s', (_name, affine, regions) => {
      const { scene, raw, draws } = makeScene({ listing: listing(), regions: regions() })
      commitSelectionTransform(scene, [box, di(60n, CTX_UP)], affine)
      expect(draws[0]).toBe('remove:32,33')
      expect(raw.draft_remove_edges.mock.invocationCallOrder[0]).toBeLessThan(
        raw.transform_selection.mock.invocationCallOrder[0],
      )
      expect(raw.transform_selection.mock.invocationCallOrder[0]).toBeLessThan(
        raw.draft_add_segment.mock.invocationCallOrder[0],
      )
      expect(segs(draws)).toHaveLength(2)
    })
  })

  describe('curve runs are re-bracketed when drawn again', () => {
    /** The top face part (30, 31 on the box) and a free HALF circle: three
     *  facets of one circle run, centered (1, 0.5, 1), radius 0.5. */
    const halfCircle = (kind: 'circle' | 'polygon' = 'circle'): Record<string, FakeUnit[]> => ({
      [CTX_UP]: [
        {
          key: 60n,
          edges: [
            { key: 30n, a: [0, 0, 1], b: [1, 0, 1], store: TOPO1 },
            { key: 31n, a: [1, 0, 1], b: [1, 1, 1], store: TOPO1 },
            { key: 40n, a: [1.5, 0.5, 1], b: [1.35, 0.85, 1], store: FREE3, curve: { center: [1, 0.5, 1], radius: 0.5 } },
            { key: 41n, a: [1.35, 0.85, 1], b: [1, 1, 1], store: FREE3, curve: { center: [1, 0.5, 1], radius: 0.5 } },
            { key: 42n, a: [1, 1, 1], b: [0, 1, 1], store: FREE3 },
          ],
          curves: [{ key: 98n, curveKey: 99n, kind, edges: [40n, 41n], center: [1, 0.5, 1], radius: 0.5 }],
        },
      ],
    })

    it('keeps a circle half a circle under a similarity: mapped center, radius times the scale', () => {
      const { scene, draws } = makeScene({ listing: halfCircle() })
      // Uniform x2 about the origin, in the plane z = 1 (a similarity there).
      const scale2 = new Float64Array([2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1, 0])
      commitSelectionTransform(scene, [box, di(60n, CTX_UP)], scale2, null, 'Scale')
      const at = draws.findIndex((d) => d.startsWith('circle:'))
      expect(draws.slice(at, at + 4)).toEqual([
        'circle:2,1,1:1',
        'seg:3,1,1>2.7,1.7,1',
        'seg:2.7,1.7,1>2,2,1',
        'end_curve',
      ])
      // The plain remainder follows the run.
      expect(draws.slice(at + 4)).toEqual(['seg:2,2,1>0,2,1'])
    })

    it('a rotation keeps the radius; a polygon run re-brackets as a polygon', () => {
      const { scene, draws } = makeScene({ listing: halfCircle('polygon') })
      const quarter = affineToFloat64(rotateAboutPivotAxis(0, 0, 1, 0, 0, 1, Math.PI / 2))
      commitSelectionTransform(scene, [box, di(60n, CTX_UP)], quarter, null, 'Rotate')
      const open = draws.find((d) => d.startsWith('polygon:'))
      expect(open).toBe('polygon:-0.5,1,1:0.5')
      expect(draws.some((d) => d.startsWith('circle:'))).toBe(false)
    })

    it('a non-uniform scale (not a similarity) draws the run as a plain chain', () => {
      const { scene, draws } = makeScene({ listing: halfCircle() })
      const stretch = new Float64Array([2, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])
      commitSelectionTransform(scene, [box, di(60n, CTX_UP)], stretch, null, 'Scale')
      expect(draws.some((d) => d.startsWith('circle:'))).toBe(false)
      const at = draws.indexOf('curve')
      expect(at).toBeGreaterThan(-1)
      expect(draws[at + 3]).toBe('end_curve')
    })
  })

  it('a refusal while redrawing aborts the whole gesture: the document is restored, nothing is undone by hand', () => {
    const before = JSON.stringify(listing(), (_k, v) => (typeof v === 'bigint' ? v.toString() : v))
    const { scene, raw, listing: live } = makeScene({ listing: listing(), failAddAt: 1 })
    expect(() => commitSelectionTransform(scene, [box, di(60n, CTX_UP)], AFFINE)).toThrow(/WouldRetopologize/)
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    expect(raw.history_bracket_abort).toHaveBeenCalledTimes(1)
    expect(raw.history_bracket_commit).not.toHaveBeenCalled()
    expect(raw.scene_undo).not.toHaveBeenCalled()
    expect(JSON.stringify(live, (_k, v) => (typeof v === 'bigint' ? v.toString() : v))).toBe(before)
  })

  it('Select All shape (object + mixed shape + a pure ground shape): one bracket, one commit labeled for the tool, drafts in both planes', () => {
    const { scene, raw } = makeScene({ listing: { ...listing(), [CTX]: [chain(40n, [7n])] } })
    commitSelectionTransform(scene, [box, di(60n, CTX_UP), di(40n)], AFFINE, null, 'Move')
    expect(raw.history_bracket_open).toHaveBeenCalledTimes(1)
    expect(raw.history_bracket_commit).toHaveBeenCalledTimes(1)
    expect(raw.history_bracket_commit).toHaveBeenCalledWith('Move')
    expect(raw.draft_end.mock.calls.length).toBe(3) // erase, pure move, redraw
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
  })

  it('a gesture that is a single kernel step skips the bracket', () => {
    const { scene, raw } = makeScene({ listing: { [CTX]: [chain(40n, [7n])] } })
    commitSelectionTransform(scene, [di(40n)], AFFINE)
    expect(raw.history_bracket_open).not.toHaveBeenCalled()
  })

  it('a pure-void group (not mixed) still drafts BEFORE the object, as always', () => {
    const { scene, raw } = makeScene({ listing: { ...listing(), [CTX]: [chain(40n, [7n])] } })
    commitSelectionTransform(scene, [box, di(40n)], AFFINE)
    expect(raw.draft_transform_edges.mock.invocationCallOrder[0]).toBeLessThan(
      raw.transform_selection.mock.invocationCallOrder[0],
    )
  })

  it('a vertical move succeeds: the free half detaches after the object, no NotInPlane', () => {
    const { scene, raw } = makeScene({ listing: listing(), islands: { '3': [60n, 61n] } })
    expect(() => commitSelectionTransform(scene, [box, di(60n, CTX_UP)], AFFINE_UP)).not.toThrow()
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
    expect(raw.transform_sketch_islands).toHaveBeenCalledTimes(1)
    expect(raw.transform_selection.mock.invocationCallOrder[0]).toBeLessThan(
      raw.transform_sketch_islands.mock.invocationCallOrder[0],
    )
  })

  it('a vertical move whose free half is a whole sketch bakes it after the object', () => {
    const { scene, raw, calls } = makeScene({ listing: listing(), islands: { '3': [60n] } })
    commitSelectionTransform(scene, [box, di(60n, CTX_UP)], AFFINE_UP)
    const sel = calls.filter((c) => c.name === 'transform_selection')
    expect(sel).toHaveLength(2)
    expect([...(sel[0].args[2] as BigUint64Array)]).toEqual([])
    expect([...(sel[1].args[2] as BigUint64Array)]).toEqual([3n])
    expect(raw.transform_sketch_islands).not.toHaveBeenCalled()
  })

  describe('copy', () => {
    /** Copies draw the captured non-riding edges per rep (no removal), after
     *  the objects copy; the drawing riding the objects goes with them. */
    const west = affineToFloat64(translationAffine(-1.5, 0, 0))
    const segs = (draws: string[]) => draws.filter((d) => d.startsWith('seg:'))

    it('copies the object FIRST, then draws only the non-riding edges at the rep, erasing nothing', () => {
      const { scene, raw, draws } = makeScene({ listing: listing() })
      const copyOwners = vi.fn(() => [{ kind: 'object', id: 9n } as NodeRef])
      const out = copyOwnersAndDrawing(scene, [box], [di(60n, CTX_UP)], [west], copyOwners)
      expect(out[out.length - 1]).toEqual({ kind: 'object', id: 9n })
      expect(copyOwners.mock.invocationCallOrder[0]).toBeLessThan(raw.draft_add_segment.mock.invocationCallOrder[0])
      expect(raw.draft_remove_edges).not.toHaveBeenCalled()
      expect(raw.draft_copy_edges).not.toHaveBeenCalled()
      expect(segs(draws)).toEqual(['seg:-0.5,1,1>0.5,1,1', 'seg:0.5,1,1>0.5,0,1'])
      // Drawing + objects = one bracket, labeled Copy.
      expect(raw.history_bracket_open).toHaveBeenCalledTimes(1)
      expect(raw.history_bracket_commit).toHaveBeenCalledWith('Copy')
    })

    it('an array draws every rep inside the one draft, keeping the shapes whole on every copy', () => {
      const { scene, raw, draws } = makeScene({ listing: listing() })
      const reps = [1, 2, 3].map((k) => affineToFloat64(translationAffine(3 * k, 0, 0)))
      copyOwnersAndDrawing(scene, [box], [di(60n, CTX_UP)], reps, () => [])
      expect(segs(draws)).toHaveLength(6)
      expect(raw.draft_end).toHaveBeenCalledTimes(1) // all reps in ONE draft
      expect(raw.history_bracket_commit).toHaveBeenCalledTimes(1)
    })

    it('a refusal while drawing a copy aborts the bracket (the object copy is restored, nothing undone by hand)', () => {
      const { scene, raw } = makeScene({ listing: listing(), failAddAt: 0 })
      const copyOwners = vi.fn(() => [] as NodeRef[])
      expect(() => copyOwnersAndDrawing(scene, [box], [di(60n, CTX_UP)], [west], copyOwners)).toThrow(/WouldRetopologize/)
      expect(copyOwners).toHaveBeenCalledTimes(1)
      expect(raw.history_bracket_abort).toHaveBeenCalledTimes(1)
      expect(raw.history_bracket_commit).not.toHaveBeenCalled()
      expect(raw.scene_undo).not.toHaveBeenCalled()
    })

    it('whole shapes copy before the owners; a selection with no owners copies just the drawing', () => {
      const { scene, raw } = makeScene({ listing: listing() })
      const copyOwners = vi.fn(() => [] as NodeRef[])
      copyOwnersAndDrawing(scene, [], [di(60n, CTX_UP)], [AFFINE], copyOwners)
      expect(copyOwners).not.toHaveBeenCalled()
      expect(raw.draft_copy_edges).toHaveBeenCalled()
      expect(raw.history_bracket_open).not.toHaveBeenCalled() // a single draft is one step
    })
  })
})

describe('draftEdgesToErase (Delete)', () => {
  const group: NodeRef = { kind: 'group', id: 5n }
  const listing = () => ({
    [CTX_UP]: [chain(60n, [30n, 31n], [0, 0, 1], TOPO1)],
    [CTX_INST]: [chain(61n, [32n], [0, 0, 0], { kind: 'owned', object: 7, sketch: 9 })],
    [CTX]: [chain(40n, [7n])],
  })

  it('erases a lone face shape, ground drawing, and nothing covered by an object in the delete', () => {
    const { scene } = makeScene({ listing: listing() })
    const lone = draftEdgesToErase(scene, [di(60n, CTX_UP), di(40n)], new Set())
    expect([...lone.draftEdgeKeys.get(CTX_UP)!]).toEqual([hex(30n), hex(31n)])
    expect([...lone.draftEdgeKeys.get(CTX)!]).toEqual([hex(7n)])
    const covered = draftEdgesToErase(scene, [{ kind: 'object', id: 1n }, di(60n, CTX_UP)], new Set())
    expect(covered.draftEdgeKeys.size).toBe(0)
  })

  it('treats an object inside a selected group as covered', () => {
    const { scene, raw } = makeScene({ listing: listing() })
    raw.group_members.mockImplementation(((g: bigint) =>
      g === 5n ? [{ kind: 'object', id: 1n }] : []) as never)
    const r = draftEdgesToErase(scene, [group, di(60n, CTX_UP)], new Set())
    expect(r.draftEdgeKeys.size).toBe(0)
  })

  it('treats drawing in a selected instance\'s context as covered, never stripping the definition', () => {
    const { scene } = makeScene({ listing: listing() })
    const inst: NodeRef = { kind: 'instance', id: INSTANCE }
    expect(draftEdgesToErase(scene, [inst, di(61n, CTX_INST)], new Set()).draftEdgeKeys.size).toBe(0)
    // An unselected instance's drawing is erased as before.
    expect(draftEdgesToErase(scene, [di(61n, CTX_INST)], new Set()).draftEdgeKeys.size).toBe(1)
  })

  it('erases only the non-covered edges of a mixed shape, and a deleted sketch covers its free edges', () => {
    const { scene } = makeScene({
      listing: {
        [CTX_UP]: [
          {
            key: 60n,
            edges: [
              { key: 30n, a: [0, 0, 1], b: [1, 0, 1], store: TOPO1 },
              { key: 32n, a: [1, 0, 1], b: [2, 0, 1], store: FREE3 },
            ],
          },
        ],
      },
    })
    const withBox = draftEdgesToErase(scene, [{ kind: 'object', id: 1n }, di(60n, CTX_UP)], new Set())
    expect([...withBox.draftEdgeKeys.get(CTX_UP)!]).toEqual([hex(32n)])
    expect([...withBox.huskCandidates]).toEqual([3n])
    const withSketch = draftEdgesToErase(scene, [{ kind: 'sketch', id: 3n }, di(60n, CTX_UP)], new Set([3n]))
    expect([...withSketch.draftEdgeKeys.get(CTX_UP)!]).toEqual([hex(30n)])
  })
})
