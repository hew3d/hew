/**
 * planeUnits — the app's view of the drawing on a plane: canonical context
 * strings, the generation-keyed listing cache, and the expansion of a
 * selection ref (island, curve, edge, region) into its edge keys, geometry,
 * stores, and objects.
 *
 * A plain-object scene double stands in for the wasm `Scene` (the module only
 * imports its type), serving a hand-written `plane_units_json` listing.
 */
import { describe, expect, it, vi } from 'vitest'
import { regionUnderRay } from './regionPick'
import {
  allFree,
  canonicalPlane,
  ctxGround,
  ctxOfFace,
  ctxOfPlane,
  ctxOfSketch,
  ctxOfSketchOnly,
  ctxOfSketchUnits,
  ctxThroughAffine,
  planeOfCtxIn,
  detachedSketchIds,
  sketchOfCtx,
  edgesOfCurveRef,
  runsOfCurve,
  unitName,
  widenCurveRef,
  draftRef,
  draftRefAlive,
  edgeKeysOf,
  geometryOf,
  invalidatePlaneUnits,
  setKernelHidden,
  keyToId,
  keysToArray,
  numberedLabels,
  objectsOf,
  outlinerDrawing,
  parsePlaneUnits,
  pickDraftRef,
  planeOfCtx,
  readPlaneUnits,
  regionRingEdges,
  resolveDraft,
  storesOf,
  unitOfRegion,
  unitsAtSegments,
} from './planeUnits'
import type { Scene as WasmScene } from '../wasm/loader'

const K = (n: number) => n.toString(16).padStart(16, '0')

type E = {
  key: string
  a: number[]
  b: number[]
  store: Record<string, unknown>
  curve?: { center: number[]; radius: number } | null
}
const edge = (n: number, a: number[], b: number[], store: Record<string, unknown>, curve: E['curve'] = null): E => ({
  key: K(n), a, b, store, curve,
})

/** Rectangle (0,0)-(1,1) on a box top (z = 0.5): four topology edges, one unit. */
const TOPO = { kind: 'topology', object: 7 }
const RECT: E[] = [
  edge(1, [0, 0, 0.5], [1, 0, 0.5], TOPO),
  edge(2, [1, 0, 0.5], [1, 1, 0.5], TOPO),
  edge(3, [1, 1, 0.5], [0, 1, 0.5], TOPO),
  edge(4, [0, 1, 0.5], [0, 0, 0.5], TOPO),
]
const FREE = { kind: 'free', sketch: 3 }
const OWNED = { kind: 'owned', object: 7, sketch: 4 }

type Run = { key: number; curveKey: number; kind?: 'circle' | 'polygon'; edges: number[]; center?: number[]; radius?: number }
function listing(units: { key: number; edges: E[]; curves?: Run[] }[], regions: unknown[] = []): string {
  return JSON.stringify({
    units: units.map((u) => ({
      key: K(u.key),
      edges: u.edges,
      curves: (u.curves ?? []).map((r) => ({
        key: K(r.key),
        curve_key: K(r.curveKey),
        kind: r.kind ?? 'circle',
        center: r.center ?? [5, 5, 0.5],
        radius: r.radius ?? 1,
        edges: r.edges.map(K),
      })),
    })),
    regions,
  })
}

function makeScene(overrides: Record<string, unknown> = {}) {
  let gen = 1n
  const plane_units_json = vi.fn((ctx: string) => {
    void ctx
    return listing([])
  })
  const scene = {
    plane_units_json,
    pick_plane_unit: vi.fn(() => 'null'),
    history_generation: () => gen,
    ...overrides,
  }
  return {
    scene: scene as unknown as WasmScene,
    raw: scene as typeof scene & { plane_units_json: typeof plane_units_json },
    bump: () => { gen += 1n },
  }
}

const CTX = ctxOfPlane([0, 0, 0.5], [0, 0, 1])!

describe('canonical plane context', () => {
  it('is the plane form with the closest point to the origin and a unit normal', () => {
    expect(JSON.parse(ctxOfPlane([3, 4, 0.5], [0, 0, 2])!)).toEqual({ plane: [0, 0, 0.5, 0, 0, 1] })
    expect(canonicalPlane([1, 1, 1], [0, 0, 0])).toBeNull()
  })

  it('names one plane by one string however it is reached, with -0 folded', () => {
    // The ground from three origins and a scaled normal.
    expect(ctxOfPlane([5, -2, 0], [0, 0, 1])).toBe(ctxGround())
    expect(ctxOfPlane([0, 0, 0], [0, -0, 9])).toBe(ctxGround())
    expect(ctxGround()).toBe('{"plane":[0,0,0,0,0,1]}')
    // A tilted plane: the origin moves along the normal to its closest point.
    const s = Math.SQRT1_2
    const a = JSON.parse(ctxOfPlane([2, 0, 0], [s, 0, s])!).plane as number[]
    const b = JSON.parse(ctxOfPlane([0, 0, 2], [s, 0, s])!).plane as number[]
    expect(a).toEqual(b)
  })

  it('opposite normals are different contexts (a face keeps its outward normal)', () => {
    expect(ctxOfPlane([0, 0, 0], [0, 0, -1])).not.toBe(ctxGround())
  })

  it('adds the instance inside an entered component instance', () => {
    expect(JSON.parse(ctxOfPlane([0, 0, 1], [0, 0, 1], 12n)!)).toEqual({ plane: [0, 0, 1, 0, 0, 1], instance: 12 })
    expect(planeOfCtx(ctxOfPlane([0, 0, 1], [0, 0, 1], 12n)!)?.instance).toBe(12n)
    expect(planeOfCtx(CTX)?.instance).toBeNull()
  })

  it('a face and the sketch lying on it give the same context', () => {
    const { scene } = makeScene({
      face_plane: () => [0.3, 0.3, 0.5, 0, 0, 1],
      face_normal: () => [0, 0, 1],
      sketch_plane: () => new Float64Array([0.9, 0.1, 0.5, 0, 0, 1]),
    })
    expect(ctxOfFace(scene, 1n, 2n)).toBe(CTX)
    expect(ctxOfSketch(scene, 5n)).toBe(CTX)
  })

  it('poses a definition sketch to world through the instance', () => {
    const pose = new Float64Array([1, 0, 0, 10, 0, 1, 0, 0, 0, 0, 1, 0])
    const { scene } = makeScene({
      sketch_plane: () => new Float64Array([0, 0, 0.5, 0, 0, 1]),
      instance_pose: () => pose,
    })
    expect(JSON.parse(ctxOfSketch(scene, 5n, 9n)!)).toEqual({ plane: [0, 0, 0.5, 0, 0, 1], instance: 9 })
    expect(ctxOfSketch(makeScene({ sketch_plane: () => undefined }).scene, 5n)).toBeNull()
  })

  it('moves a plane through an affine (the plane an out-of-plane move lands on)', () => {
    const lift = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 2]
    expect(ctxThroughAffine(ctxGround(), lift)).toBe(ctxOfPlane([0, 0, 2], [0, 0, 1]))
  })

  it('maps a slanted plane by the inverse transpose under a non-uniform scale, and keeps a mirror facing out', () => {
    // The plane x + z = 1 holds (1,0,0) and (0,0,1); stretched 2x in x they
    // land at (2,0,0) and (0,0,1): the plane x/2 + z = 1, normal ∝ (1,0,2).
    const stretch = [2, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]
    const got = JSON.parse(ctxThroughAffine(ctxOfPlane([1, 0, 0], [1, 0, 1])!, stretch)!).plane as number[]
    const want = JSON.parse(ctxOfPlane([2, 0, 0], [1, 0, 2])!).plane as number[]
    got.forEach((v, i) => expect(v).toBeCloseTo(want[i], 9))
    // Mirrored in x, an x-facing plane faces -x (its outside goes with it).
    const mirror = [-1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]
    expect(ctxThroughAffine(ctxOfPlane([1, 0, 0], [1, 0, 0])!, mirror)).toBe(ctxOfPlane([-1, 0, 0], [-1, 0, 0]))
  })
})

describe('listing cache', () => {
  it('parses once per document generation and reparses after a change', () => {
    const { scene, raw, bump } = makeScene()
    readPlaneUnits(scene, CTX)
    readPlaneUnits(scene, CTX)
    expect(raw.plane_units_json).toHaveBeenCalledTimes(1)
    bump() // a committed mutation, undo, or redo
    readPlaneUnits(scene, CTX)
    expect(raw.plane_units_json).toHaveBeenCalledTimes(2)
  })

  it('is keyed per context and dropped by invalidatePlaneUnits', () => {
    const { scene, raw } = makeScene()
    readPlaneUnits(scene, CTX)
    readPlaneUnits(scene, ctxGround())
    expect(raw.plane_units_json).toHaveBeenCalledTimes(2)
    invalidatePlaneUnits()
    readPlaneUnits(scene, CTX)
    expect(raw.plane_units_json).toHaveBeenCalledTimes(3)
  })

  it('a hide/unhide (setKernelHidden) drops the cache even though history did not move', () => {
    const { scene, raw } = makeScene({ set_hidden: vi.fn() })
    readPlaneUnits(scene, CTX)
    readPlaneUnits(scene, CTX)
    expect(raw.plane_units_json).toHaveBeenCalledTimes(1)
    setKernelHidden(scene as unknown as Parameters<typeof setKernelHidden>[0], [7n], [])
    expect((raw as unknown as { set_hidden: ReturnType<typeof vi.fn> }).set_hidden).toHaveBeenCalledTimes(1)
    readPlaneUnits(scene, CTX)
    expect(raw.plane_units_json).toHaveBeenCalledTimes(2)
  })

  it('does not cache a scene that cannot report its generation', () => {
    const { scene, raw } = makeScene({ history_generation: undefined })
    readPlaneUnits(scene, CTX)
    readPlaneUnits(scene, CTX)
    expect(raw.plane_units_json).toHaveBeenCalledTimes(2)
  })

  it('answers null when the kernel refuses the listing', () => {
    const { scene } = makeScene({ plane_units_json: () => { throw new Error('BadContext: no') } })
    expect(readPlaneUnits(scene, CTX)).toBeNull()
  })
})

/** A scene whose plane lists one rectangle, one circle drawn in two runs (a
 *  line crosses it: two separate pieces sharing one curve key), and one lone
 *  free line. */
function drawnScene() {
  const circle = { center: [5, 5, 0.5], radius: 1 }
  const curveKey = 0xc0ffee
  const units = [
    { key: 1, edges: RECT },
    {
      key: 10,
      edges: [
        edge(10, [6, 5, 0.5], [5, 6, 0.5], OWNED, circle),
        edge(11, [5, 6, 0.5], [4, 5, 0.5], OWNED, circle),
        edge(12, [4, 5, 0.5], [5, 4, 0.5], OWNED, circle),
      ],
      curves: [
        { key: 0xa1, curveKey, edges: [10, 11] },
        { key: 0xa2, curveKey, edges: [12] },
      ],
    },
    { key: 20, edges: [edge(20, [8, 8, 0.5], [9, 8, 0.5], FREE)] },
  ]
  const regions = [
    {
      key: K(1),
      outer: [[0, 0, 0.5], [1, 0, 0.5], [1, 1, 0.5], [0, 1, 0.5]],
      holes: [],
      over: { object: 7, face: 30 },
      extrude: null,
    },
    {
      key: K(99),
      outer: [[-2, -2, 0.5], [3, -2, 0.5], [3, 3, 0.5], [-2, 3, 0.5]],
      holes: [[[0, 0, 0.5], [1, 0, 0.5], [1, 1, 0.5], [0, 1, 0.5]]],
      over: { object: 7, face: 31 },
      extrude: null,
    },
  ]
  return {
    ...makeScene({ plane_units_json: () => listing(units, regions), pick_plane_unit: vi.fn(() => 'null') }),
    runKey: K(0xa1),
    curveKey: K(curveKey),
  }
}

describe('expanding a ref', () => {
  it('an island is all of its edge keys; an edge is itself', () => {
    const { scene } = drawnScene()
    expect(edgeKeysOf(scene, draftRef('draft-island', CTX, K(1)))).toEqual([K(1), K(2), K(3), K(4)])
    expect(edgeKeysOf(scene, draftRef('draft-edge', CTX, K(2)))).toEqual([K(2)])
    expect(edgeKeysOf(scene, draftRef('draft-island', CTX, K(777)))).toEqual([]) // stale
  })

  it('a curve ref is a RUN: its edges only; the circle’s own curve key names every run', () => {
    const { scene, runKey, curveKey } = drawnScene()
    expect(edgeKeysOf(scene, draftRef('draft-curve', CTX, runKey))).toEqual([K(10), K(11)])
    expect(edgeKeysOf(scene, draftRef('draft-curve', CTX, K(0xa2)))).toEqual([K(12)])
    // Widened (double-click): the whole circle, every run.
    expect(edgeKeysOf(scene, draftRef('draft-curve', CTX, curveKey))).toEqual([K(10), K(11), K(12)])
    expect(edgeKeysOf(scene, draftRef('draft-curve', CTX, K(5)))).toEqual([])
    const pu = readPlaneUnits(scene, CTX)!
    expect(runsOfCurve(pu, curveKey).map((r) => r.key)).toEqual([K(0xa1), K(0xa2)])
    expect(edgesOfCurveRef(pu, runKey).map((e) => e.key)).toEqual([K(10), K(11)])
    expect(pu.runOfEdge.get(K(12))?.key).toBe(K(0xa2))
    expect(pu.runOfEdge.has(K(1))).toBe(false) // a plain line has no run
  })

  it('widenCurveRef turns a run of a cut-apart circle into the whole circle, and leaves anything else', () => {
    const { scene, runKey, curveKey } = drawnScene()
    expect(widenCurveRef(scene, draftRef('draft-curve', CTX, runKey))).toEqual(draftRef('draft-curve', CTX, curveKey))
    const whole = draftRef('draft-curve', CTX, curveKey)
    expect(widenCurveRef(scene, whole)).toBe(whole)
    const line = draftRef('draft-edge', CTX, K(1))
    expect(widenCurveRef(scene, line)).toBe(line)
    // A one-edge run (picked as a line) of that cut-apart curve widens too.
    expect(widenCurveRef(scene, draftRef('draft-edge', CTX, K(12)))).toEqual(draftRef('draft-curve', CTX, curveKey))
    const plain = draftRef('draft-edge', CTX, K(20))
    expect(widenCurveRef(scene, plain)).toBe(plain)
    const one = makeScene({ plane_units_json: () => listing([{ key: 1, edges: RECT, curves: [{ key: 9, curveKey: 8, edges: [1, 2] }] }]) })
    const r = draftRef('draft-curve', CTX, K(9))
    expect(widenCurveRef(one.scene, r)).toBe(r)
  })

  it('a region is its drawing ring edges; the solid-bounded background has none on its outer ring', () => {
    const { scene } = drawnScene()
    const rect = draftRef('draft-region', CTX, K(1))
    expect(edgeKeysOf(scene, rect).sort()).toEqual([K(1), K(2), K(3), K(4)])
    const pu = readPlaneUnits(scene, CTX)!
    expect(unitOfRegion(pu, pu.regionByKey.get(K(1))!)?.key).toBe(K(1))
    // The face around the shape is background, not a shape.
    expect(unitOfRegion(pu, pu.regionByKey.get(K(99))!)).toBeNull()
    // …but its HOLE ring is the rectangle's edges.
    expect(regionRingEdges(pu, pu.regionByKey.get(K(99))!).map((e) => e.key).sort()).toEqual([K(1), K(2), K(3), K(4)])
  })

  it('a fill click selects a shape only when the region is free or every outer-ring edge is a drawing edge', () => {
    const face = { object: 7, face: 30 }
    const ring = (pts: number[][]) => pts.map((p) => [p[0], p[1], 0.5])
    // Face (0,0)-(4,4); rectangle (0,0)-(1,1) flush to the corner: only its
    // two interior sides are drawing edges, the other two lie on the solid's.
    const flushUnits = [{ key: 1, edges: [
      edge(2, [1, 0, 0.5], [1, 1, 0.5], TOPO),
      edge(3, [1, 1, 0.5], [0, 1, 0.5], TOPO),
    ] }]
    // A free rectangle (over no face) and an inner sub-face rectangle.
    const freeUnits = [{ key: 5, edges: [
      edge(51, [10, 10, 0.5], [11, 10, 0.5], FREE),
      edge(52, [11, 10, 0.5], [11, 11, 0.5], FREE),
      edge(53, [11, 11, 0.5], [10, 11, 0.5], FREE),
      edge(54, [10, 11, 0.5], [10, 10, 0.5], FREE),
    ] }]
    const regions = [
      { key: K(100), outer: ring([[0, 0], [1, 0], [1, 1], [0, 1]]), holes: [], over: face, extrude: null },
      { key: K(101), outer: ring([[1, 0], [4, 0], [4, 4], [0, 4], [0, 1], [1, 1]]), holes: [], over: face, extrude: null },
      { key: K(102), outer: ring([[10, 10], [11, 10], [11, 11], [10, 11]]), holes: [], over: null, extrude: null },
    ]
    const { scene } = makeScene({ plane_units_json: () => listing([...flushUnits, ...freeUnits], regions) })
    const pu = readPlaneUnits(scene, CTX)!
    expect(unitOfRegion(pu, pu.regionByKey.get(K(100))!)).toBeNull() // rectangle fill, ring has solid edges
    expect(unitOfRegion(pu, pu.regionByKey.get(K(101))!)).toBeNull() // bare L
    expect(unitOfRegion(pu, pu.regionByKey.get(K(102))!)?.key).toBe(K(5)) // free island
    // The inner rectangle over a face (existing fixture) is a shape.
    const d = drawnScene()
    const dpu = readPlaneUnits(d.scene, CTX)!
    expect(unitOfRegion(dpu, dpu.regionByKey.get(K(1))!)?.key).toBe(K(1))
    // Through the pick path: both flush fills resolve to the object (null ref).
    for (const key of [K(100), K(101)]) {
      ;(scene as unknown as { pick_plane_unit: unknown }).pick_plane_unit = () => JSON.stringify({ region: key })
      expect(pickDraftRef(scene, CTX, { origin: [0.5, 0.5, 4], direction: [0, 0, -1] }, 0.01)).toBeNull()
    }
  })

  it('geometryOf is flat world segments', () => {
    const { scene } = drawnScene()
    expect(Array.from(geometryOf(scene, draftRef('draft-edge', CTX, K(20))))).toEqual([8, 8, 0.5, 9, 8, 0.5])
    expect(geometryOf(scene, draftRef('draft-island', CTX, K(1)))).toHaveLength(24)
  })

  it('reports stores and objects, including a mixed island', () => {
    const mixed = [
      edge(1, [0, 0, 0.5], [1, 0, 0.5], TOPO),
      edge(2, [1, 0, 0.5], [2, 0, 0.5], FREE),
    ]
    const { scene } = makeScene({ plane_units_json: () => listing([{ key: 1, edges: mixed }]) })
    const ref = draftRef('draft-island', CTX, K(1))
    expect(storesOf(scene, ref)).toEqual([
      { kind: 'topology', object: 7n },
      { kind: 'free', sketch: 3n },
    ])
    expect(objectsOf(scene, ref)).toEqual([7n])
    expect(allFree(scene, ref)).toBe(false)
    const { scene: s2 } = drawnScene()
    expect(allFree(s2, draftRef('draft-island', CTX, K(20)))).toBe(true)
    expect(objectsOf(s2, draftRef('draft-island', CTX, K(10)))).toEqual([7n])
  })

  it('a ref is alive while its key is listed', () => {
    const { scene, curveKey } = drawnScene()
    expect(draftRefAlive(scene, draftRef('draft-island', CTX, K(1)))).toBe(true)
    expect(draftRefAlive(scene, draftRef('draft-island', CTX, K(2)))).toBe(false)
    expect(draftRefAlive(scene, draftRef('draft-edge', CTX, K(2)))).toBe(true)
    expect(draftRefAlive(scene, draftRef('draft-region', CTX, K(1)))).toBe(true)
    expect(draftRefAlive(scene, draftRef('draft-curve', CTX, curveKey))).toBe(true)
    expect(draftRefAlive(scene, { kind: 'object', id: 1n })).toBe(false)
  })

  it('carries the hex key as the id', () => {
    const r = draftRef('draft-island', CTX, K(255))
    expect(r.id).toBe(255n)
    expect(keyToId(K(255))).toBe(255n)
    expect(Array.from(keysToArray([K(1), K(2)]))).toEqual([1n, 2n])
  })

  it('finds units by edge positions, either direction', () => {
    const { scene } = drawnScene()
    const pu = readPlaneUnits(scene, CTX)!
    expect(unitsAtSegments(pu, [[[1, 0.5 * 0 + 0, 0.5], [0, 0, 0.5]]]).map((u) => u.key)).toEqual([K(1)])
    expect(unitsAtSegments(pu, [[[50, 0, 0.5], [51, 0, 0.5]]])).toEqual([])
  })
})

describe('picking', () => {
  it('turns an edge pick into a line, a piece of a circle into its run, a region fill into its shape', () => {
    const edgePick = vi.fn(() => JSON.stringify({ edge: K(20), unit: K(20), curve: null }))
    const { scene } = drawnScene()
    ;(scene as unknown as { pick_plane_unit: unknown }).pick_plane_unit = edgePick
    const ray = { origin: [8.5, 8, 4], direction: [0, 0, -1] }
    expect(pickDraftRef(scene, CTX, ray, 0.01)).toEqual(draftRef('draft-edge', CTX, K(20)))

    ;(scene as unknown as { pick_plane_unit: unknown }).pick_plane_unit = () =>
      JSON.stringify({ edge: K(10), unit: K(10), curve: K(0xc0ffee), run: K(0xa1) })
    expect(pickDraftRef(scene, CTX, ray, 0.01)).toEqual(draftRef('draft-curve', CTX, K(0xa1)))
    // A run of ONE facet is just a line.
    ;(scene as unknown as { pick_plane_unit: unknown }).pick_plane_unit = () =>
      JSON.stringify({ edge: K(12), unit: K(10), curve: K(0xc0ffee), run: K(0xa2) })
    expect(pickDraftRef(scene, CTX, ray, 0.01)).toEqual(draftRef('draft-edge', CTX, K(12)))

    ;(scene as unknown as { pick_plane_unit: unknown }).pick_plane_unit = () => JSON.stringify({ region: K(1) })
    expect(pickDraftRef(scene, CTX, ray, 0.01)).toEqual(draftRef('draft-island', CTX, K(1)))

    // The face around a shape: background, nothing to select.
    ;(scene as unknown as { pick_plane_unit: unknown }).pick_plane_unit = () => JSON.stringify({ region: K(99) })
    expect(pickDraftRef(scene, CTX, ray, 0.01)).toBeNull()
    ;(scene as unknown as { pick_plane_unit: unknown }).pick_plane_unit = () => 'null'
    expect(pickDraftRef(scene, CTX, ray, 0.01)).toBeNull()
  })

  it('has no JS fallback: a refusing kernel pick selects nothing', () => {
    const { scene } = drawnScene()
    ;(scene as unknown as { pick_plane_unit: unknown }).pick_plane_unit = () => {
      throw new Error('refused')
    }
    const ray = { origin: [8.5, 8, 4], direction: [0, 0, -1] }
    expect(pickDraftRef(scene, CTX, ray, 0.01)).toBeNull()
  })
})

describe('parsePlaneUnits', () => {
  it('reads a null circle radius (an ellipse in a stretched instance) as null', () => {
    const json = JSON.stringify({
      units: [{
        key: K(1),
        edges: [{ key: K(2), a: [0, 0, 0], b: [1, 0, 0], store: { kind: 'free', sketch: 1 }, curve: { center: [0, 0, 0], radius: null } }],
        curves: [{ key: K(3), curve_key: K(4), kind: 'circle', center: [0, 0, 0], radius: null, edges: [K(2)] }],
      }],
      regions: [],
    })
    const pu = parsePlaneUnits(CTX, json)
    expect(pu.units[0].edges[0].curve).toEqual({ center: [0, 0, 0], radius: null })
    expect(pu.units[0].curves[0].radius).toBeNull()
  })

  it('skips malformed entries instead of throwing', () => {
    const pu = parsePlaneUnits(CTX, JSON.stringify({ units: [{ edges: [] }, { key: K(1), edges: [{ key: K(2) }] }], regions: [{}] }))
    expect(pu.units.map((u) => u.key)).toEqual([K(1)])
    expect(pu.units[0].edges).toEqual([])
    expect(pu.regions).toEqual([])
    expect(parsePlaneUnits(CTX, 'not json').units).toEqual([])
  })
})

describe('outlinerDrawing placement', () => {
  /** Plane A (box top): a unit on object 7's topology, a unit mixing topology
   *  and free, a unit in an owned sketch. Plane B (ground): one free unit. */
  function placementScene() {
    const ground = ctxGround()
    const lists: Record<string, string> = {
      [CTX]: listing([
        { key: 1, edges: RECT },
        { key: 40, edges: [edge(40, [0, 0, 0.5], [1, 0, 0.5], TOPO), edge(41, [1, 0, 0.5], [2, 0, 0.5], FREE)] },
        { key: 50, edges: [edge(50, [0, 3, 0.5], [1, 3, 0.5], OWNED)] },
      ]),
      [ground]: listing([{ key: 60, edges: [edge(60, [0, 0, 0], [1, 0, 0], { kind: 'free', sketch: 9 })] }]),
    }
    return makeScene({
      sketch_ids: () => [3n, 9n],
      sketch_plane: (s: bigint) => (s === 9n ? new Float64Array([0, 0, 0, 0, 0, 1]) : new Float64Array([0, 0, 0.5, 0, 0, 1])),
      plane_units_json: (ctx: string) => lists[ctx] ?? listing([]),
    })
  }

  it('lists topology, owned, and mixed units under their object, free-only units as sketch rows', () => {
    const { scene } = placementScene()
    const d = outlinerDrawing(scene as never, [7n])
    expect((d.byObject.get('7') ?? []).map((u) => u.unit.key)).toEqual([K(1), K(40), K(50)])
    expect(d.free.map((u) => u.unit.key)).toEqual([K(60)])
    expect(d.free[0].ref).toEqual(draftRef('draft-island', ctxGround(), K(60)))
  })

  it('names an object’s shapes from their edges and curve runs, numbering repeats', () => {
    const { scene } = placementScene()
    const d = outlinerDrawing(scene as never, [7n])
    const refs = (d.byObject.get('7') ?? []).map((u) => u.ref)
    // A 4-sided right-angled loop; a two-edge open path; a lone segment.
    expect(refs.map((r) => d.labels.get(`${r.kind}:${r.ctx}:${r.key}`))).toEqual(['Rectangle', 'Edge shape', 'Line'])
    expect(numberedLabels(['Circle', 'Circle', 'Line'])).toEqual(['Circle 1', 'Circle 2', 'Line'])
  })

  it('reads the free drawing in one call when the host offers it, without a read per plane', () => {
    const ground = ctxGround()
    const groundList = listing([{ key: 60, edges: [edge(60, [0, 0, 0], [1, 0, 0], { kind: 'free', sketch: 9 })] }])
    const perPlane: string[] = []
    const { scene } = makeScene({
      sketch_ids: () => [9n],
      sketch_plane: () => new Float64Array([0, 0, 0, 0, 0, 1]),
      history_generation: () => 1n,
      plane_units_all_json: () => JSON.stringify([{ plane: [0, 0, 0, 0, 0, 1], units: JSON.parse(groundList) }]),
      plane_units_json: (ctx: string) => { perPlane.push(ctx); return listing([]) },
    })
    const d = outlinerDrawing(scene as never, [])
    expect(d.free.map((u) => u.unit.key)).toEqual([K(60)])
    expect(d.free[0].ref).toEqual(draftRef('draft-island', ground, K(60)))
    expect(perPlane).toEqual([])
  })

  it('finds a topology-only drawing (no sketch) through its object’s drawing planes', () => {
    const { scene } = makeScene({
      sketch_ids: () => [],
      object_drawing_planes: () => new Float64Array([0, 0, 0.5, 0, 0, 1]),
      plane_units_json: () => listing([{ key: 1, edges: RECT }]),
    })
    const d = outlinerDrawing(scene as never, [7n])
    expect((d.byObject.get('7') ?? []).map((u) => u.unit.key)).toEqual([K(1)])
    expect(d.free).toEqual([])
  })

  it('resolveDraft lifts nothing by itself: an edge ref resolves to its one edge and its unit', () => {
    const { scene } = drawnScene()
    const r = resolveDraft(scene, draftRef('draft-edge', CTX, K(3)))!
    expect(r.edges.map((e) => e.key)).toEqual([K(3)])
    expect(r.unit?.key).toBe(K(1))
  })
})

describe('unitName', () => {
  const ring = (n: number, r = 1): E[] =>
    Array.from({ length: n }, (_, i) => {
      const a = (i / n) * 2 * Math.PI, b = ((i + 1) / n) * 2 * Math.PI
      return edge(100 + i, [r * Math.cos(a), r * Math.sin(a), 0], [r * Math.cos(b), r * Math.sin(b), 0], FREE, { center: [0, 0, 0], radius: r })
    })
  const unit = (edges: E[], curves: Run[] = []) =>
    parsePlaneUnits(CTX, listing([{ key: 1, edges, curves }])).units[0]

  it('names a lone whole circle, an arc, and a regular polygon', () => {
    const edges = ring(24)
    expect(unitName(unit(edges, [{ key: 1, curveKey: 2, edges: edges.map((_, i) => 100 + i) }]))).toBe('Circle')
    const arc = edges.slice(0, 8)
    expect(unitName(unit(arc, [{ key: 1, curveKey: 2, edges: arc.map((_, i) => 100 + i) }]))).toBe('Arc')
    const hex = ring(6)
    expect(unitName(unit(hex, [{ key: 1, curveKey: 2, kind: 'polygon', edges: hex.map((_, i) => 100 + i) }]))).toBe('Polygon')
  })

  it('names rectangles, lines, other loops, and arc shapes', () => {
    expect(unitName(unit(RECT))).toBe('Rectangle')
    expect(unitName(unit([RECT[0]]))).toBe('Line')
    const tri = [
      edge(1, [0, 0, 0], [1, 0, 0], FREE), edge(2, [1, 0, 0], [0, 1, 0], FREE), edge(3, [0, 1, 0], [0, 0, 0], FREE),
    ]
    expect(unitName(unit(tri))).toBe('Shape')
    // A rectangle whose corner is rounded by a circle run: an arc shape.
    const mixed = [...RECT.slice(0, 3), edge(9, [0, 1, 0.5], [0, 0, 0.5], FREE, { center: [0, 0, 0.5], radius: 1 })]
    expect(unitName(unit(mixed, [{ key: 5, curveKey: 6, edges: [9, 2] }]))).toBe('Arc shape')
  })
})

// Detached shapes (docs/design/separate-shapes.md §6.6): a detached sketch is
// its own drafting context, `{"sketch":id}`, handed back by the kernel pick.
describe('detached shapes', () => {
  const SEP = { kind: 'free', sketch: 5, detached: true }
  const SCTX = ctxOfSketchOnly(5)
  const sepListing = listing([
    { key: 70, edges: [edge(70, [0, 0, 0], [1, 0, 0], SEP), edge(71, [1, 0, 0], [0, 1, 0], SEP), edge(72, [0, 1, 0], [0, 0, 0], SEP)] },
  ], [{ key: K(8), outer: [[0, 0, 0], [1, 0, 0], [0, 1, 0]], holes: [], over: null, extrude: { sketch: 5, region: 6 } }])

  it('names a detached sketch by one canonical context string', () => {
    expect(SCTX).toBe('{"sketch":5}')
    expect(ctxOfSketchOnly(5n, 9n)).toBe('{"sketch":5,"instance":9}')
    expect(sketchOfCtx(SCTX)).toEqual({ sketch: 5n, instance: null })
    expect(sketchOfCtx(ctxGround())).toBeNull()
  })

  it('reads the detached flag off a free store', () => {
    const pu = parsePlaneUnits(SCTX, sepListing)
    const store = pu.units[0].edges[0].store
    expect(store).toEqual({ kind: 'free', sketch: 5n, detached: true })
    // An ordinary free store carries no flag.
    expect(parsePlaneUnits(CTX, listing([{ key: 1, edges: [edge(1, [0, 0, 0], [1, 0, 0], FREE)] }])).units[0].edges[0].store)
      .toEqual({ kind: 'free', sketch: 3n })
  })

  it('a pick that names a detached sketch makes refs in THAT context; a plain pick keeps the request context', () => {
    const lists: Record<string, string> = { [SCTX]: sepListing }
    const { scene } = makeScene({
      plane_units_json: (ctx: string) => lists[ctx] ?? listing([]),
    })
    const ray = { origin: [0.2, 0.2, 4], direction: [0, 0, -1] }
    const raw = scene as unknown as { pick_plane_unit: unknown }
    raw.pick_plane_unit = () => JSON.stringify({ edge: K(70), unit: K(70), curve: null, run: null, ctx: { sketch: 5 } })
    expect(pickDraftRef(scene, ctxGround(), ray, 0.01)).toEqual(draftRef('draft-edge', SCTX, K(70)))
    raw.pick_plane_unit = () => JSON.stringify({ region: K(8), ctx: { sketch: 5 } })
    expect(pickDraftRef(scene, ctxGround(), ray, 0.01)).toEqual(draftRef('draft-island', SCTX, K(70)))
    raw.pick_plane_unit = () => JSON.stringify({ edge: K(70), unit: K(70), curve: null, run: null, ctx: { sketch: 5, instance: 2 } })
    expect(pickDraftRef(scene, ctxGround(), ray, 0.01)).toEqual(draftRef('draft-edge', ctxOfSketchOnly(5, 2), K(70)))
    raw.pick_plane_unit = () => JSON.stringify({ edge: K(70), unit: K(70), curve: null, run: null })
    expect(pickDraftRef(scene, ctxGround(), ray, 0.01)).toEqual(draftRef('draft-edge', ctxGround(), K(70)))
  })

  it('is free drawing: allFree holds and its context names the sketch plane', () => {
    const { scene } = makeScene({
      sketch_plane: () => new Float64Array([0, 0, 2, 0, 0, 1]),
      plane_units_json: () => sepListing,
    })
    const ref = draftRef('draft-island', SCTX, K(70))
    expect(allFree(scene, ref)).toBe(true)
    expect(planeOfCtx(SCTX)).toBeNull()
    expect(planeOfCtxIn(scene as never, SCTX)).toEqual({ point: [0, 0, 2], normal: [0, 0, 1], instance: null })
  })

  describe('listing', () => {
    const groundList = listing([{ key: 60, edges: [edge(60, [0, 0, 0], [1, 0, 0], { kind: 'free', sketch: 9 })] }])
    const batch = () => JSON.stringify([
      { plane: [0, 0, 0, 0, 0, 1], units: JSON.parse(groundList) },
      { plane: [0, 0, 0, 0, 0, 1], sketch: 5, detached: true, units: JSON.parse(sepListing) },
    ])

    it('Outliner lists a detached entry as its own context, a plain free row like any other', () => {
      const perPlane: string[] = []
      const { scene } = makeScene({
        sketch_ids: () => [9n, 5n],
        sketch_plane: () => new Float64Array([0, 0, 0, 0, 0, 1]),
        plane_units_all_json: batch,
        plane_units_json: (ctx: string) => { perPlane.push(ctx); return listing([]) },
      })
      const d = outlinerDrawing(scene as never, [])
      expect(d.free.map((u) => u.unit.key)).toEqual([K(60), K(70)])
      const sep = d.free[1]
      expect(sep.ref).toEqual(draftRef('draft-island', SCTX, K(70)))
      // Both free rows keep the "Sketch N" fallback: no label, no "detached"
      // suffix (a standalone row already says it is not attached).
      for (const u of d.free) expect(d.labels.has(`${u.ref.kind}:${u.ref.ctx}:${u.ref.key}`)).toBe(false)
      expect(perPlane).toEqual([])
    })

    it('finds the detached sketches (marquee, Select All, tools) from the same one read', () => {
      const all = vi.fn(batch)
      const { scene } = makeScene({
        sketch_plane: () => new Float64Array([0, 0, 0, 0, 0, 1]),
        plane_units_all_json: all,
      })
      expect([...detachedSketchIds(scene as never)]).toEqual([5n])
      expect(ctxOfSketchUnits(scene as WasmScene, 5n)).toBe(SCTX)
      // A sketch in a plane's arrangement answers its plane's context.
      expect(ctxOfSketchUnits(scene as WasmScene, 9n)).toBe(ctxOfSketch(scene as WasmScene, 9n))
      expect(all).toHaveBeenCalledTimes(1)
    })
  })
})

describe('regionUnderRay on a detached shape', () => {
  it('resolves the region in the pick\'s own sketch context, not the request context', () => {
    const own = ctxOfSketchOnly(9n)
    const region = { key: K(50), outer: [[0, 0, 0.5], [1, 0, 0.5], [1, 1, 0.5], [0, 1, 0.5]], holes: [], over: { object: 7, face: 31 }, extrude: null }
    const { scene } = makeScene({
      // The request's (face) context lists no region at all; the detached sketch's own does.
      plane_units_json: (ctx: string) => (ctx === own ? listing([], [region]) : listing([])),
      pick_plane_unit: () => JSON.stringify({ region: K(50), ctx: { sketch: 9 } }),
      sketch_plane: () => new Float64Array([0, 0, 0.5, 0, 0, 1]),
    })
    const hit = regionUnderRay(scene as never, CTX, { origin: [0.5, 0.5, 4], direction: [0, 0, -1] }, 0)
    expect(hit).not.toBeNull()
    expect(hit?.ctx).toBe(own)
    expect(hit?.region.key).toBe(K(50))
  })
})
