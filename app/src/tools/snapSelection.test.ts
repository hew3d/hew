/**
 * snapSelection — the single, complete resolver both the Select click and the
 * drag-move arm reduce to. These tests drive `resolveSelectableRef` across the
 * matrix of (provenance × editing-context × depth), which IS the click/drag
 * parity: both callers pass their snap+ray through this one function, so a
 * shared result here is a shared result there. classifySnapPick is pinned too
 * (the provenance axis).
 *
 * Everything DRAWN resolves through one path: the snap or ray names a plane
 * and the kernel's `pick_plane_unit` says which drawing unit is under the ray.
 * The scene double below models that with a tiny geometric simulator over a
 * per-plane listing (`plane_units_json` shape), so the assertions stay about
 * behaviour — which ref, which context, which tolerance.
 */
import { describe, it, expect, vi } from 'vitest'
import { classifySnapPick, resolveSelectableRef, type ResolveDeps, type SelectScene } from './snapSelection'
import { ctxGround, ctxOfPlane, edgeKeysOf, planeOfCtx, widenCurveRef, type PlaneUnitsScene } from './planeUnits'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

const base = { x: 0, y: 0, z: 0 }
const solidSnap = (object: bigint, instance?: bigint): Snap => ({ ...base, kind: 'on-face', object, instance })
const regionSnap = (sketch: bigint, region: bigint): Snap => ({ ...base, kind: 'on-face', elementKind: 'sketch-region', sketch, sketchRegion: region })
const edgeSnap = (sketch: bigint, edge: bigint): Snap => ({ ...base, kind: 'on-edge', elementKind: 'sketch-edge', sketch, element: edge })
/** A drawn curve's analytic point (center / quadrant / tangent / facet vertex
 * / polygon center): names the CHAIN, carries no `element`. `kind` is cosmetic
 * here — the resolver keys off `elementKind` + `sketchCurve`. */
const curveSnap = (sketch: bigint, curve: bigint, kind = 'center'): Snap => ({ ...base, kind, elementKind: 'sketch-curve', sketch, sketchCurve: curve })
/** A curve-provenance snap resolved AT a specific world point — the shape a
 * real click near one fragment of a multi-fragment curve produces: the
 * analytic candidate's own position, close to the cursor. */
const curveSnapAt = (sketch: bigint, curve: bigint, point: readonly [number, number, number], kind = 'center'): Snap => ({
  ...curveSnap(sketch, curve, kind),
  x: point[0],
  y: point[1],
  z: point[2],
})
const bareSnap = (): Snap => ({ ...base, kind: 'endpoint' })

/** A ray straight down -Z (dead-centre) unless a direction is given. */
const rayDir = (d: [number, number, number]): Ray => ({ origin: [0, 0, 5], direction: d })
const DOWN = rayDir([0, 0, -1])
const FORWARD: [number, number, number] = [0, 0, -1] // camera looks -Z

// ── A plane-unit listing and a simulated kernel pick over it ────────────────

type Pt = [number, number, number]
/** 16-hex kernel key of a small number. */
const K = (n: number): string => n.toString(16).padStart(16, '0')
const ID = (key: string): bigint => BigInt('0x' + key)

type FStore =
  | { kind: 'free'; sketch: number }
  | { kind: 'topology'; object: number }
  | { kind: 'owned'; object: number; sketch: number }
interface FEdge {
  key: string
  a: Pt
  b: Pt
  store?: FStore
  /** An analytic circle facet: its center/radius and the kernel curve key. */
  curve?: { center: Pt; radius: number; key: string; run?: string; kind?: 'circle' | 'polygon' }
}
interface FUnit { key: string; edges: FEdge[] }
interface FRegion { key: string; outer: Pt[]; holes?: Pt[][]; over?: { object: number; face: number | null } | null }
interface Listing { units: FUnit[]; regions: FRegion[] }

const FREE_21: FStore = { kind: 'free', sketch: 21 }

/** The four edges of an axis-aligned rectangle on plane z, keys base..base+3. */
function rectEdges(z: number, half: number, base_: number, store: FStore = FREE_21, cx = 0, cy = 0): FEdge[] {
  const c: Pt[] = [
    [cx - half, cy - half, z], [cx + half, cy - half, z], [cx + half, cy + half, z], [cx - half, cy + half, z],
  ]
  return c.map((p, i) => ({ key: K(base_ + i), a: p, b: c[(i + 1) % 4], store }))
}
const rectRing = (z: number, half: number, cx = 0, cy = 0): Pt[] => [
  [cx - half, cy - half, z], [cx + half, cy - half, z], [cx + half, cy + half, z], [cx - half, cy + half, z],
]
/** One rectangle drawn on plane z: a unit and its fill region. */
function rectListing(z: number, opts: { store?: FStore; unit?: number; region?: number; half?: number } = {}): Listing {
  const half = opts.half ?? 1
  return {
    units: [{ key: K(opts.unit ?? 900), edges: rectEdges(z, half, 100, opts.store ?? FREE_21) }],
    regions: [{ key: K(opts.region ?? 5), outer: rectRing(z, half) }],
  }
}
/** A circle of `n` facets, edge keys base..base+n-1, one curve key. */
function circleEdges(z: number, n: number, base_: number, curveKey: string, store: FStore = FREE_21, c: Pt = [0, 0, z], r = 1): FEdge[] {
  return Array.from({ length: n }, (_v, i) => {
    const t0 = (2 * Math.PI * i) / n
    const t1 = (2 * Math.PI * (i + 1)) / n
    return {
      key: K(base_ + i),
      a: [c[0] + r * Math.cos(t0), c[1] + r * Math.sin(t0), z] as Pt,
      b: [c[0] + r * Math.cos(t1), c[1] + r * Math.sin(t1), z] as Pt,
      store,
      curve: { center: c, radius: r, key: curveKey },
    }
  })
}

const ptSegDist = (p: Pt, a: Pt, b: Pt): number => {
  const ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
  const l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]
  const t = l2 > 0 ? Math.max(0, Math.min(1, ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1] + (p[2] - a[2]) * ab[2]) / l2)) : 0
  return Math.hypot(p[0] - (a[0] + ab[0] * t), p[1] - (a[1] + ab[1] * t), p[2] - (a[2] + ab[2] * t))
}
const inPoly = (x: number, y: number, ring: Pt[]): boolean => {
  let inside = false
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i]
    const [xj, yj] = ring[j]
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside
  }
  return inside
}
const ringArea = (ring: Pt[]): number => {
  let a = 0
  for (let i = 0; i < ring.length; i++) {
    const p = ring[i], q = ring[(i + 1) % ring.length]
    a += p[0] * q[1] - q[0] * p[1]
  }
  return Math.abs(a) / 2
}

/** The run key a facet of curve `curveKey` gets by default: one run per
 *  (curve, unit) — a circle a line crosses is several runs sharing a curve. */
const runKeyOf = (curveKey: string, unitIndex = 0): string => K(Number(ID(curveKey)) + 0x10000 * (unitIndex + 1))

/** The `curves` runs of a unit: its curve-tagged edges grouped by run. */
function runsJson(u: FUnit, unitIndex: number): unknown[] {
  const runs = new Map<string, { curve: NonNullable<FEdge['curve']>; edges: string[] }>()
  for (const e of u.edges) {
    if (!e.curve) continue
    const rk = e.curve.run ?? runKeyOf(e.curve.key, unitIndex)
    const r = runs.get(rk) ?? { curve: e.curve, edges: [] }
    r.edges.push(e.key)
    runs.set(rk, r)
  }
  return [...runs].map(([key, r]) => ({
    key,
    curve_key: r.curve.key,
    kind: r.curve.kind ?? 'circle',
    center: r.curve.center,
    radius: r.curve.radius,
    edges: r.edges,
  }))
}

const listingJson = (l: Listing): string =>
  JSON.stringify({
    units: l.units.map((u, ui) => ({
      key: u.key,
      curves: runsJson(u, ui),
      edges: u.edges.map((e) => ({
        key: e.key,
        a: e.a,
        b: e.b,
        store: e.store ?? FREE_21,
        curve: e.curve ? { center: e.curve.center, radius: e.curve.radius } : null,
      })),
    })),
    regions: l.regions.map((r) => ({
      key: r.key,
      outer: r.outer,
      holes: r.holes ?? [],
      over: r.over ?? null,
      extrude: null,
    })),
  })

/** The kernel's `pick_plane_unit` over a listing, on the context's plane:
 *  the nearest drawing edge within `tol` (with its unit, curve key, and run), else
 *  the smallest region containing the point, else `null`. */
function simulatePick(l: Listing, ctx: string, ray: Float64Array, tol: number): string {
  const pl = planeOfCtx(ctx)
  if (pl === null) return 'null'
  const [ox, oy, oz, dx, dy, dz] = ray
  const den = dx * pl.normal[0] + dy * pl.normal[1] + dz * pl.normal[2]
  if (Math.abs(den) < 1e-12) return 'null'
  const t = ((pl.point[0] - ox) * pl.normal[0] + (pl.point[1] - oy) * pl.normal[1] + (pl.point[2] - oz) * pl.normal[2]) / den
  const p: Pt = [ox + dx * t, oy + dy * t, oz + dz * t]
  let best: { e: FEdge; u: FUnit; ui: number; d: number } | null = null
  l.units.forEach((u, ui) => {
    for (const e of u.edges) {
      const d = ptSegDist(p, e.a, e.b)
      if (d <= tol && (best === null || d < best.d)) best = { e, u, ui, d }
    }
  })
  if (best !== null) {
    const b = best as { e: FEdge; u: FUnit; ui: number; d: number }
    const run = b.e.curve ? (b.e.curve.run ?? runKeyOf(b.e.curve.key, b.ui)) : null
    return JSON.stringify({ edge: b.e.key, unit: b.u.key, curve: b.e.curve?.key ?? null, run })
  }
  let reg: FRegion | null = null
  for (const r of l.regions) {
    if (!inPoly(p[0], p[1], r.outer)) continue
    if (r.holes?.some((h) => inPoly(p[0], p[1], h))) continue
    if (reg === null || ringArea(r.outer) < ringArea(reg.outer)) reg = r
  }
  return reg === null ? 'null' : JSON.stringify({ region: reg.key })
}

interface SceneStubs {
  region?: { sketch: bigint; region: bigint; depth?: number }
  edge?: { sketch: bigint; edge: bigint; depth?: number }
  face?: { object: bigint; instance?: bigint; depth: number; face?: bigint }
  /** The drawing on each plane, keyed by the plane's z (all planes are
   *  horizontal, normal +Z). z = 0 is the ground and every sketch's plane. */
  drawn?: Record<number, Listing>
  /** The z of every solid face (default 3 — off every listed plane). */
  faceZ?: number
  /** World endpoints `[ax,ay,az,bx,by,bz]` per sketch-edge handle, or
   *  `undefined` for an edge the double doesn't model — omit entirely to
   *  simulate a host with no `sketch_edge_endpoints` at all. */
  edgeEndpoints?: (edge: bigint) => number[] | undefined
  instanceRegion?: { sketch: bigint; region: bigint; depth?: number }
  instanceEdge?: { sketch: bigint; edge: bigint; depth?: number }
  /** Omit plane_units_json / pick_plane_unit entirely (a host with no plane
   *  listing: nothing drawn can ever resolve). */
  noPlanes?: boolean
  /** Every plane listing / pick the resolver made. */
  log?: { ctx: string; tol?: number }[]
}

const IDENTITY_POSE = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]

function fakeScene(s: SceneStubs): SelectScene {
  const zOfCtx = (ctx: string): number => planeOfCtx(ctx)?.point[2] ?? NaN
  const listingOf = (ctx: string): Listing => s.drawn?.[zOfCtx(ctx)] ?? { units: [], regions: [] }
  const faceZ = s.faceZ ?? 3
  return {
    ...(s.edgeEndpoints ? { sketch_edge_endpoints: (_sk: bigint, edge: bigint) => s.edgeEndpoints!(edge) } : {}),
    sketch_plane: () => new Float64Array([0, 0, 0, 0, 0, 1]),
    instance_pose: () => new Float64Array(IDENTITY_POSE),
    face_plane: () => new Float64Array([0, 0, faceZ, 0, 0, 1]),
    face_normal: () => new Float64Array([0, 0, 1]),
    ...(s.noPlanes
      ? {}
      : {
          plane_units_json: (ctx: string) => {
            s.log?.push({ ctx })
            return listingJson(listingOf(ctx))
          },
          pick_plane_unit: (ctx: string, ray: Float64Array, tol: number) => {
            s.log?.push({ ctx, tol })
            return simulatePick(listingOf(ctx), ctx, ray, tol)
          },
        }),
    pick_sketch_region: () =>
      s.region && {
        sketch: () => s.region!.sketch,
        region: () => s.region!.region,
        depth: () => s.region!.depth ?? 5,
        free: vi.fn(),
      },
    pick_sketch_edge: () =>
      s.edge && {
        sketch: () => s.edge!.sketch,
        edge: () => s.edge!.edge,
        depth: () => s.edge!.depth ?? 5,
        free: vi.fn(),
      },
    pick_sketch_region_in_instance: () =>
      s.instanceRegion && {
        sketch: () => s.instanceRegion!.sketch,
        region: () => s.instanceRegion!.region,
        depth: () => s.instanceRegion!.depth ?? 5,
        free: vi.fn(),
      },
    pick_sketch_edge_in_instance: () =>
      s.instanceEdge && {
        sketch: () => s.instanceEdge!.sketch,
        edge: () => s.instanceEdge!.edge,
        depth: () => s.instanceEdge!.depth ?? 5,
        free: vi.fn(),
      },
    pick_face: () =>
      s.face && {
        object: () => s.face!.object,
        face: () => s.face!.face,
        instance: () => s.face!.instance,
        depth: () => s.face!.depth,
        free: vi.fn(),
      },
  } as unknown as SelectScene
}

function deps(
  scene: SelectScene,
  context: NodeRef[],
  far = 100,
  worldPerPixel = WPP,
  imprintEligible?: ResolveDeps['imprintEligible'],
): ResolveDeps {
  return {
    scene,
    context,
    // The Viewport's resolveObject is context-scoped; the stub tags the object
    // so tests can tell it apart from a sketch ref.
    resolveObject: (object, instance) => ({ kind: 'object', id: object, instance } as unknown as NodeRef),
    cameraForward: FORWARD,
    cameraFar: far,
    worldPerPixelAt: () => worldPerPixel,
    ...(imprintEligible ? { imprintEligible } : {}),
  }
}

/** A working-zoom camera for a metre-scale model: 0.25 mm per pixel, so the
 *  8 px pick reaches 2 mm. */
const WPP = 2.5e-4

const GROUND = ctxGround()
const draft = (kind: NodeRef['kind'], ctx: string, key: string): NodeRef => ({ kind, id: ID(key), ctx, key })

const GROUP_CTX: NodeRef[] = [{ kind: 'group', id: 1n } as unknown as NodeRef]
const INSTANCE_CTX: NodeRef[] = [{ kind: 'instance', id: 7n }]
const INSTANCE_PLANE = ctxOfPlane([0, 0, 0], [0, 0, 1], 7n) as string

describe('classifySnapPick', () => {
  it('maps each provenance (and null) to its kind', () => {
    expect(classifySnapPick(solidSnap(7n, 9n))).toEqual({ kind: 'object', object: 7n, instance: 9n })
    expect(classifySnapPick(regionSnap(21n, 5n))).toEqual({ kind: 'sketch-region', sketch: 21n, region: 5n })
    expect(classifySnapPick(edgeSnap(11n, 4n))).toEqual({ kind: 'sketch-edge', sketch: 11n, edge: 4n })
    expect(classifySnapPick(curveSnap(11n, 8n))).toEqual({ kind: 'sketch-curve', sketch: 11n, curve: 8n })
    expect(classifySnapPick(bareSnap())).toEqual({ kind: 'fallback' })
    expect(classifySnapPick(null)).toEqual({ kind: 'fallback' })
  })

  it('a sketch-curve snap missing its curve handle degrades to fallback, never a half-built pick', () => {
    // Defensive: element_kind said "sketch-curve" but the handle is absent.
    expect(classifySnapPick({ ...base, kind: 'center', elementKind: 'sketch-curve', sketch: 11n })).toEqual({ kind: 'fallback' })
  })
})


/** Owned by object 42 — what a definition-owned sketch's drawing is. */
const OWNED_42: FStore = { kind: 'owned', object: 42, sketch: 31 }
const eligible42: ResolveDeps['imprintEligible'] = (o) => o === 42n
const lineThroughOrigin = (key: number, store: FStore): Listing => ({
  units: [{ key: K(950), edges: [{ key: K(key), a: [-1, 0, 0], b: [1, 0, 0], store }] }],
  regions: [],
})

describe('resolveSelectableRef — provenance × context × depth', () => {
  it('a solid snap → the object node, at top level and in a context', () => {
    const scene = fakeScene({})
    expect(resolveSelectableRef(solidSnap(7n, 9n), DOWN, deps(scene, []))).toMatchObject({ kind: 'object', id: 7n })
    expect(resolveSelectableRef(solidSnap(7n), DOWN, deps(scene, GROUP_CTX))).toMatchObject({ kind: 'object', id: 7n })
  })

  it('a region-fill snap → its shape (island) at top level, on the canonical ground context', () => {
    const scene = fakeScene({ drawn: { 0: rectListing(0) } })
    const ref = resolveSelectableRef(regionSnap(21n, 5n), DOWN, deps(scene, []))
    expect(ref).toEqual(draft('draft-island', GROUND, K(900)))
    expect(ref?.ctx).toBe('{"plane":[0,0,0,0,0,1]}')
  })

  it('a region-fill snap INSIDE a context is out of scope → the in-context solid, never the sketch (FIX 1)', () => {
    // The top-level sketch region is not selectable in-context; the fallback
    // resolves the in-context solid under the ray instead.
    const scene = fakeScene({ drawn: { 0: rectListing(0) }, face: { object: 42n, depth: 8 } })
    const ref = resolveSelectableRef(regionSnap(21n, 5n), DOWN, deps(scene, GROUP_CTX))
    expect(ref).toMatchObject({ kind: 'object', id: 42n })
  })

  it('a sketch-edge snap → the edge (or its curve) at top level', () => {
    const edgeScene = (listing: Listing) =>
      fakeScene({ drawn: { 0: listing }, edgeEndpoints: (e) => (e === 4n ? [-1, 0, 0, 1, 0, 0] : undefined) })
    expect(resolveSelectableRef(edgeSnap(11n, 4n), DOWN, deps(edgeScene(lineThroughOrigin(4, FREE_21)), [])))
      .toEqual(draft('draft-edge', GROUND, K(4)))
    // The same endpoints on a facet of a multi-facet circle name its RUN.
    const curve = { center: [0, 1, 0] as Pt, radius: 1, key: K(77) }
    const circle: Listing = {
      units: [{ key: K(800), edges: [
        { key: K(4), a: [-1, 0, 0], b: [1, 0, 0], curve },
        { key: K(5), a: [1, 0, 0], b: [-1, 0, 0], curve },
      ] }],
      regions: [],
    }
    expect(resolveSelectableRef(edgeSnap(11n, 4n), DOWN, deps(edgeScene(circle), [])))
      .toEqual(draft('draft-curve', GROUND, runKeyOf(K(77))))
    // A run of ONE facet is just a line, as a one-edge chain always was.
    const single: Listing = {
      units: [{ key: K(800), edges: [{ key: K(4), a: [-1, 0, 0], b: [1, 0, 0], curve }] }],
      regions: [],
    }
    expect(resolveSelectableRef(edgeSnap(11n, 4n), DOWN, deps(edgeScene(single), [])))
      .toEqual(draft('draft-edge', GROUND, K(4)))
  })

  it('a sketch-edge snap INSIDE a context is out of scope → the in-context solid, never the sketch (FIX 1)', () => {
    const scene = fakeScene({ drawn: { 0: lineThroughOrigin(4, FREE_21) }, face: { object: 42n, depth: 8 } })
    expect(resolveSelectableRef(edgeSnap(11n, 4n), DOWN, deps(scene, GROUP_CTX))).toMatchObject({ kind: 'object', id: 42n })
  })

  it('a provenance-less snap falls back to region → edge → solid, top-level only for sketches', () => {
    // Region under the ray wins.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ drawn: { 0: rectListing(0) }, region: { sketch: 21n, region: 5n } }), [])))
      .toEqual(draft('draft-island', GROUND, K(900)))
    // No region → a sketch edge.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ drawn: { 0: lineThroughOrigin(4, FREE_21) }, edge: { sketch: 11n, edge: 4n } }), [])))
      .toEqual(draft('draft-edge', GROUND, K(4)))
    // No sketch geometry → a near solid.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ face: { object: 42n, depth: 50 } }), []))).toMatchObject({ kind: 'object', id: 42n })
    // Genuinely empty → null.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({}), []))).toBeNull()
    // In a context, sketch pickers are skipped — only the in-context solid.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ drawn: { 0: rectListing(0) }, region: { sketch: 21n, region: 5n }, face: { object: 42n, depth: 8 } }), GROUP_CTX))).toMatchObject({ kind: 'object', id: 42n })
  })

  it('a line lying on an axis is picked at a screen distance where the ray meets it, not at the camera', () => {
    // The axis snap outranks the line and names nothing to select, so the
    // click falls back to the edge under the ray. A perspective camera's
    // scale shrinks to zero at the camera itself: measured there, the 8 px
    // reach would vanish and only a click exactly on the line would hit it.
    const onAxis: Snap = { ...base, kind: 'on-axis', x: 0.5, y: 0, z: 0 }
    const ray: Ray = { origin: [0.5, 0.001, 5], direction: [0, 0, -1] } // 1 mm (4 px) off the line
    const d: ResolveDeps = {
      ...deps(fakeScene({ drawn: { 0: lineThroughOrigin(4, FREE_21) }, edge: { sketch: 11n, edge: 4n } }), []),
      worldPerPixelAt: (p) => (WPP / 5) * Math.hypot(p[0] - ray.origin[0], p[1] - ray.origin[1], p[2] - ray.origin[2]),
    }
    expect(resolveSelectableRef(onAxis, ray, d)).toEqual(draft('draft-edge', GROUND, K(4)))
  })

  it('a guide or axis snap on a drawn line selects the line wherever the snap holds, not only within the pick radius', () => {
    // Guides and axes outrank a line lying along them, so the snap names the
    // guide while the cursor is still well off the line: the click selects
    // what the snap latched onto, the line under the snap point.
    for (const kind of ['on-guide', 'on-axis']) {
      const snap: Snap = { ...base, kind, x: 0.5, y: 0, z: 0 }
      const ray: Ray = { origin: [0.5, 0.0075, 5], direction: [0, 0, -1] } // 30 px off the line
      const scene = fakeScene({ drawn: { 0: lineThroughOrigin(4, FREE_21) }, edge: { sketch: 11n, edge: 4n } })
      expect(resolveSelectableRef(snap, ray, deps(scene, []))).toEqual(draft('draft-edge', GROUND, K(4)))
    }
  })

  it('inside an instance, fallback picks its posed definition sketch before the member solid', () => {
    const regionScene = fakeScene({
      drawn: { 0: rectListing(0, { store: OWNED_42 }) },
      instanceRegion: { sketch: 31n, region: 5n },
      face: { object: 42n, instance: 7n, depth: 8 },
    })
    // The context is the entered instance's plane (posed through it).
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(regionScene, INSTANCE_CTX, 100, WPP, eligible42)))
      .toEqual(draft('draft-island', INSTANCE_PLANE, K(900)))

    const edgeScene = fakeScene({
      drawn: { 0: lineThroughOrigin(4, OWNED_42) },
      instanceEdge: { sketch: 31n, edge: 4n },
      face: { object: 42n, instance: 7n, depth: 8 },
    })
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(edgeScene, INSTANCE_CTX, 100, WPP, eligible42)))
      .toEqual(draft('draft-edge', INSTANCE_PLANE, K(4)))
  })

  it('inside an instance the scoped probe selects any definition drawing it finds, owned or free; eligibility only gates the face refinement', () => {
    const mk = (store: FStore) => fakeScene({
      drawn: { 0: lineThroughOrigin(4, store) },
      instanceEdge: { sketch: 31n, edge: 4n },
      face: { object: 42n, instance: 7n, depth: 8 },
    })
    // Eligible owner → the drawn line.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(mk(OWNED_42), INSTANCE_CTX, 100, WPP, eligible42)))
      .toEqual(draft('draft-edge', INSTANCE_PLANE, K(4)))
    // The instance's own scoped probe is in scope by construction: eligibility
    // of the owner does not matter, and a definition sketch (free) selects too
    // (it always did — the old in-instance sketch pick had no such gate).
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(mk(OWNED_42), INSTANCE_CTX, 100, WPP, () => false)))
      .toEqual(draft('draft-edge', INSTANCE_PLANE, K(4)))
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(mk(FREE_21), INSTANCE_CTX, 100, WPP, eligible42)))
      .toEqual(draft('draft-edge', INSTANCE_PLANE, K(4)))
  })

  it('a visible definition sketch beats the member-face snap beneath it', () => {
    const scene = fakeScene({
      drawn: { 0: lineThroughOrigin(4, OWNED_42) },
      instanceEdge: { sketch: 31n, edge: 4n },
    })
    expect(
      resolveSelectableRef(
        solidSnap(42n, 7n),
        DOWN,
        deps(scene, INSTANCE_CTX, 100, WPP, eligible42),
      ),
    ).toEqual(draft('draft-edge', INSTANCE_PLANE, K(4)))
  })

  it('a definition sketch behind the nearest member face is not selectable through it', () => {
    const scene = fakeScene({
      drawn: { 0: lineThroughOrigin(4, OWNED_42) },
      instanceEdge: { sketch: 31n, edge: 4n, depth: 9 },
      face: { object: 42n, instance: 7n, depth: 5 },
    })
    expect(resolveSelectableRef(solidSnap(42n, 7n), DOWN, deps(scene, INSTANCE_CTX, 100, WPP, eligible42)))
      .toMatchObject({ kind: 'object', id: 42n })
  })

  it('in X-ray and Wireframe nothing occludes: the definition sketch behind the face is selectable', () => {
    const scene = fakeScene({
      drawn: { 0: lineThroughOrigin(4, OWNED_42) },
      instanceEdge: { sketch: 31n, edge: 4n, depth: 9 },
      face: { object: 42n, instance: 7n, depth: 5 },
    })
    const seeThrough = { ...deps(scene, INSTANCE_CTX, 100, WPP, eligible42), facesOcclude: false }
    expect(resolveSelectableRef(solidSnap(42n, 7n), DOWN, seeThrough))
      .toEqual(draft('draft-edge', INSTANCE_PLANE, K(4)))
    // Shaded, said explicitly, keeps the face in the way.
    const shaded = { ...deps(scene, INSTANCE_CTX, 100, WPP, eligible42), facesOcclude: true }
    expect(resolveSelectableRef(solidSnap(42n, 7n), DOWN, shaded)).toMatchObject({ kind: 'object', id: 42n })
  })

  it('the fallback solid pick is bounded by the AXIAL far plane, not the radial distance (FIX 2)', () => {
    // Off-centre ray (cos 0.6 with the camera forward): a VISIBLE solid at
    // RADIAL 150 sits at AXIAL 90 < far 100 → armed (the old radial-vs-far
    // bound wrongly rejected it, silently turning the drag into a marquee).
    const offCentre = rayDir([0.8, 0, -0.6]) // unit; dot with [0,0,-1] = 0.6
    const visible = fakeScene({ face: { object: 42n, depth: 150 } })
    expect(resolveSelectableRef(bareSnap(), offCentre, deps(visible, [], 100))).toMatchObject({ kind: 'object', id: 42n })

    // A genuinely-clipped solid: radial 200 → axial 120 > far 100 → rejected.
    const clipped = fakeScene({ face: { object: 42n, depth: 200 } })
    expect(resolveSelectableRef(bareSnap(), offCentre, deps(clipped, [], 100))).toBeNull()

    // Dead-centre is unchanged: radial == axial.
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ face: { object: 42n, depth: 99 } }), [], 100))).toMatchObject({ kind: 'object', id: 42n })
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(fakeScene({ face: { object: 42n, depth: 101 } }), [], 100))).toBeNull()
  })

  it('the far bound is UNIFORM: a solid beyond the render far plane is unselectable by click too, not just unmovable', () => {
    // A provenance-less snap whose nearest solid is beyond the far plane. This
    // is the exact call the Select CLICK makes (handleSelect → resolveSelectableRef).
    // Both click and drag share it, so both reject: an undrawn solid must be
    // neither selected nor moved — a selection is the precursor to Delete/Move
    // on geometry the user cannot see. Do NOT re-introduce a click-only exemption:
    // pick_face returns the NEAREST face, so a visible solid (a nearer face) is
    // never rejected here — only a pixel with no drawn solid at all.
    const clipped = deps(fakeScene({ face: { object: 42n, depth: 101 } }), [], 100)
    expect(resolveSelectableRef(bareSnap(), DOWN, clipped)).toBeNull()
    // A drawn solid at the same pixel (nearer face, within far) still selects.
    const visible = deps(fakeScene({ face: { object: 42n, depth: 99 } }), [], 100)
    expect(resolveSelectableRef(bareSnap(), DOWN, visible)).toMatchObject({ kind: 'object', id: 42n })
  })

  it('parity: click and drag get the SAME ref — they are one function of (snap, ray, deps)', () => {
    const cases: { snap: Snap | null; ray: Ray; ctx: NodeRef[]; scene: () => SelectScene }[] = [
      { snap: solidSnap(7n, 9n), ray: DOWN, ctx: [], scene: () => fakeScene({}) },
      { snap: regionSnap(21n, 5n), ray: DOWN, ctx: [], scene: () => fakeScene({ drawn: { 0: rectListing(0) } }) },
      { snap: regionSnap(21n, 5n), ray: DOWN, ctx: GROUP_CTX, scene: () => fakeScene({ face: { object: 42n, depth: 8 } }) },
      { snap: edgeSnap(11n, 4n), ray: DOWN, ctx: [], scene: () => fakeScene({ drawn: { 0: lineThroughOrigin(4, FREE_21) } }) },
      { snap: bareSnap(), ray: rayDir([0.8, 0, -0.6]), ctx: [], scene: () => fakeScene({ face: { object: 42n, depth: 150 } }) },
      { snap: null, ray: DOWN, ctx: [], scene: () => fakeScene({}) },
    ]
    for (const c of cases) {
      // The click path and the drag path each call resolveSelectableRef with
      // the same inputs → identical result, by construction (fresh scene stubs
      // per call, since the pick handles are single-use).
      const clickRef = resolveSelectableRef(c.snap, c.ray, deps(c.scene(), c.ctx))
      const dragRef = resolveSelectableRef(c.snap, c.ray, deps(c.scene(), c.ctx))
      expect(clickRef).toEqual(dragRef)
    }
  })

  it('a sketch-curve snap → the curve ref (the circle under the analytic point)', () => {
    // A circle's center lies on no edge; the resolver finds its circle from
    // the circle's own center and routes to the kernel curve key.
    // the circle's own center and routes to the run it belongs to (here the
    // whole circle is one run).
    const scene = fakeScene({ drawn: { 0: { units: [{ key: K(800), edges: circleEdges(0, 12, 30, K(77)) }], regions: [] } } })
    expect(resolveSelectableRef(curveSnap(11n, 8n), DOWN, deps(scene, []))).toEqual(draft('draft-curve', GROUND, runKeyOf(K(77))))
  })

  it('a sketch-curve snap INSIDE a context is out of scope → the in-context solid, never the curve', () => {
    const scene = fakeScene({
      drawn: { 0: { units: [{ key: K(800), edges: circleEdges(0, 12, 30, K(77)) }], regions: [] } },
      face: { object: 42n, depth: 8 },
    })
    expect(resolveSelectableRef(curveSnap(11n, 8n), DOWN, deps(scene, GROUP_CTX))).toMatchObject({ kind: 'object', id: 42n })
  })

  it('a sketch-curve snap whose circle is gone falls through to the ray re-probe, not a dead ref', () => {
    // The curve was deleted between snap and resolve: no circle edges left.
    // The resolver must not mint a ref from the stale handle — it re-probes
    // the plane under the ray (here, a shape's fill).
    const scene = fakeScene({ drawn: { 0: rectListing(0) }, region: { sketch: 21n, region: 5n } })
    expect(resolveSelectableRef(curveSnap(11n, 8n), DOWN, deps(scene, []))).toEqual(draft('draft-island', GROUND, K(900)))
  })

  it('a circle split into disjoint chains: the centre resolves to the run with the LOWEST edge key, and a rim click on each chain gives that chain\'s own run, regardless of listing order', () => {
    // A line drawn across a circle splits its facets into two units that still
    // share the curve key — two RUNS. Edges {2,3} are one run and {5,8} the
    // other. The centre lies on both; the lowest edge key breaks the tie, never
    // the listing order. A rim click selects the piece it lands on.
    const circle = circleEdges(0, 4, 0, K(77))
    const byKey = new Map<number, FEdge>([[2, circle[0]], [3, circle[1]], [5, circle[2]], [8, circle[3]]])
    const relabel = (): FUnit[] => [
      { key: K(801), edges: [{ ...circle[0], key: K(2) }, { ...circle[1], key: K(3) }] },
      { key: K(802), edges: [{ ...circle[2], key: K(5) }, { ...circle[3], key: K(8) }] },
    ]
    const endpoints = (e: bigint): number[] | undefined => {
      const f = byKey.get(Number(e))
      return f === undefined ? undefined : [...f.a, ...f.b]
    }
    const scene = (units: FUnit[]) => fakeScene({ drawn: { 0: { units, regions: [] } }, edgeEndpoints: endpoints })
    const runA = draft('draft-curve', GROUND, K(0xa01))
    const runB = draft('draft-curve', GROUND, K(0xa02))
    const withRuns = (units: FUnit[]): FUnit[] =>
      units.map((u) => ({
        ...u,
        edges: u.edges.map((e) => ({ ...e, curve: { ...e.curve!, run: u.key === K(801) ? K(0xa01) : K(0xa02) } })),
      }))
    for (const units of [relabel(), relabel().reverse(), relabel().map((u) => ({ ...u, edges: [...u.edges].reverse() }))]) {
      const sc = () => scene(withRuns(units))
      expect(resolveSelectableRef(curveSnap(11n, 8n), DOWN, deps(sc(), []))).toEqual(runA)
      // A rim click on each chain selects that chain's run.
      expect(resolveSelectableRef(edgeSnap(11n, 3n), DOWN, deps(sc(), []))).toEqual(runA)
      expect(resolveSelectableRef(edgeSnap(11n, 8n), DOWN, deps(sc(), []))).toEqual(runB)
    }
  })

  it('REGRESSION: walking a drawn circle by angle, EVERY snap the rim produces selects the SAME curve — including the centre, the quadrants, the tangents, and the facet vertices', () => {
    // The reported bug: a click that resolved to a Center or Quadrant snap
    // (and, at facet vertices, an Endpoint) carried no selectable provenance,
    // so it fell through to a ray re-probe and selected the sketch ISLAND (no
    // Segments field) instead of the curve — about half the clicks. Here one
    // drawn circle's facet edges are 30..=41; a rim edge snap names the edge
    // it hit, an analytic-point snap names the curve. Both must resolve to
    // the ONE curve ref, at every angle.
    const facets = circleEdges(0, 12, 30, K(77))
    const scene = () => fakeScene({
      drawn: { 0: { units: [{ key: K(800), edges: facets }], regions: [] } },
      edgeEndpoints: (e) => {
        const f = facets[Number(e) - 30]
        return f === undefined ? undefined : [...f.a, ...f.b]
      },
    })
    const expected = draft('draft-curve', GROUND, runKeyOf(K(77)))

    // Analytic points: centre, quadrants, tangents, and facet vertices.
    const analytic: Snap[] = [
      curveSnap(11n, 8n, 'center'),
      ...[0, 3, 6, 9].map((i) => curveSnapAt(11n, 8n, facets[i].a, 'quadrant')),
      ...[1, 7].map((i) => curveSnapAt(11n, 8n, facets[i].a, 'tangent')),
      ...facets.map((f) => curveSnapAt(11n, 8n, f.a, 'endpoint')),
    ]
    for (const snap of analytic) {
      expect(resolveSelectableRef(snap, DOWN, deps(scene(), []))).toEqual(expected)
    }
    // And a rim-EDGE snap (the ~half of clicks that always worked) lands on
    // the identical ref — so the two halves of the rim never disagree.
    for (let i = 0; i < facets.length; i++) {
      expect(resolveSelectableRef(edgeSnap(11n, BigInt(30 + i)), DOWN, deps(scene(), []))).toEqual(expected)
    }
  })

  it("Kurt's rosette: copies of a circle resolve each click to the circle under the cursor, never one on the opposite side", () => {
    // Six circles in a row (a rosette's petals, flattened), each a separate
    // circle with its own curve key; circle k has two facet edges meeting at
    // x = 10k. A click AT that shared vertex selects circle k — not the
    // lowest-key one (k=0), not a neighbour.
    const units: FUnit[] = Array.from({ length: 6 }, (_v, k) => {
      const cx = k * 10
      const curve = { center: [cx, 1, 0] as Pt, radius: 1, key: K(300 + k) }
      return {
        key: K(700 + k),
        edges: [
          { key: K(200 + 2 * k), a: [cx - 0.3, 0, 0] as Pt, b: [cx, 0, 0] as Pt, curve },
          { key: K(201 + 2 * k), a: [cx, 0, 0] as Pt, b: [cx + 0.3, 0, 0] as Pt, curve },
        ],
      }
    })
    const scene = () => fakeScene({ drawn: { 0: { units, regions: [] } } })
    for (let k = 0; k < 6; k++) {
      const clickPoint: [number, number, number] = [k * 10, 0, 0]
      const ref = resolveSelectableRef(curveSnapAt(11n, 8n, clickPoint, 'quadrant'), DOWN, deps(scene(), []))
      expect(ref).toEqual(draft('draft-curve', GROUND, runKeyOf(K(300 + k), k)))
    }
    // Explicitly: a click on the far fragment (k=3) must NOT collapse to the
    // globally-lowest curve (circle 0) the way the pre-fix resolver did.
    const oppositeSide = resolveSelectableRef(curveSnapAt(11n, 8n, [30, 0, 0], 'quadrant'), DOWN, deps(scene(), []))
    expect(oppositeSide).not.toEqual(draft('draft-curve', GROUND, runKeyOf(K(300), 0)))
  })

  it('two disconnected single-edge circle fragments: clicking each resolves to that circle, not the other', () => {
    // Each fragment is a run of ONE facet, so it selects as a plain edge.
    const units: FUnit[] = [
      { key: K(701), edges: [{ key: K(50), a: [0, 0, 0], b: [1, 0, 0], curve: { center: [0.5, 1, 0], radius: 1, key: K(310) } }] },
      { key: K(702), edges: [{ key: K(60), a: [100, 0, 0], b: [101, 0, 0], curve: { center: [100.5, 1, 0], radius: 1, key: K(311) } }] },
    ]
    const scene = () => fakeScene({ drawn: { 0: { units, regions: [] } } })
    expect(resolveSelectableRef(curveSnapAt(11n, 8n, [0.5, 0, 0], 'quadrant'), DOWN, deps(scene(), [])))
      .toEqual(draft('draft-edge', GROUND, K(50)))
    expect(resolveSelectableRef(curveSnapAt(11n, 8n, [100.5, 0, 0], 'quadrant'), DOWN, deps(scene(), [])))
      .toEqual(draft('draft-edge', GROUND, K(60)))
  })

  it('the equidistant tie (a point exactly on the shared vertex between two fragments) resolves to the LOWER edge key, deterministically regardless of listing order', () => {
    const e40: FEdge = { key: K(40), a: [-1, 0, 0], b: [0, 0, 0], curve: { center: [-1, 1, 0], radius: 1, key: K(320) } }
    const e41: FEdge = { key: K(41), a: [0, 0, 0], b: [1, 0, 0], curve: { center: [1, 1, 0], radius: 1, key: K(321) } }
    const mk = (edges: FEdge[]) => fakeScene({ drawn: { 0: { units: edges.map((e, i) => ({ key: K(710 + i), edges: [e] })), regions: [] } } })
    const tiedClick = curveSnapAt(11n, 8n, [0, 0, 0], 'center')
    expect(resolveSelectableRef(tiedClick, DOWN, deps(mk([e40, e41]), []))).toEqual(draft('draft-edge', GROUND, K(40)))
    expect(resolveSelectableRef(tiedClick, DOWN, deps(mk([e41, e40]), []))).toEqual(draft('draft-edge', GROUND, K(40)))
  })
})

describe('curve runs — what a click on a circle, arc, or polygon selects', () => {
  const OBJECT = 7n
  const topo: FStore = { kind: 'topology', object: 7 }
  const rayAt = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
  const RUN_A = K(0xa01)
  const RUN_B = K(0xa02)
  /** A 4-facet circle (curve 77) a line crosses: facets 0,1 are one run, 2,3 the other. */
  const crossedCircle = (): Listing => {
    const f = circleEdges(0, 4, 0, K(77), topo)
    const withRun = (e: FEdge, run: string): FEdge => ({ ...e, curve: { ...e.curve!, run } })
    return { units: [{ key: K(800), edges: [withRun(f[0], RUN_A), withRun(f[1], RUN_A), withRun(f[2], RUN_B), withRun(f[3], RUN_B)] }], regions: [] }
  }
  const sceneOf = (l: Listing) => fakeScene({ faceZ: 0, face: { object: OBJECT, depth: 5, face: 15n }, drawn: { 0: l } })

  it('a click on a piece of a circle crossed by a line selects THAT RUN (keyed by the run key), not the whole circle', () => {
    // Facet 0's midpoint is (0.5, 0.5); facet 2's is (-0.5, -0.5).
    expect(resolveSelectableRef(solidSnap(OBJECT), rayAt(0.5, 0.5), deps(sceneOf(crossedCircle()), [])))
      .toEqual(draft('draft-curve', GROUND, RUN_A))
    expect(resolveSelectableRef(solidSnap(OBJECT), rayAt(-0.5, -0.5), deps(sceneOf(crossedCircle()), [])))
      .toEqual(draft('draft-curve', GROUND, RUN_B))
  })

  it('a double-click widening gives the circle\'s curve key, and its edges cover every run', () => {
    const scene = sceneOf(crossedCircle())
    const run = resolveSelectableRef(solidSnap(OBJECT), rayAt(0.5, 0.5), deps(scene, [])) as NodeRef
    const runEdges = edgeKeysOf(scene as unknown as PlaneUnitsScene, run)
    expect(runEdges.sort()).toEqual([K(0), K(1)])
    const whole = widenCurveRef(scene as unknown as PlaneUnitsScene, run)
    expect(whole).toEqual(draft('draft-curve', GROUND, K(77)))
    expect(edgeKeysOf(scene as unknown as PlaneUnitsScene, whole).sort()).toEqual([K(0), K(1), K(2), K(3)])
  })

  it('a polygon run selects as ONE draft-curve over all its edges', () => {
    const sides = circleEdges(0, 6, 40, K(88), topo).map((e) => ({ ...e, curve: { ...e.curve!, kind: 'polygon' as const, run: K(0xb01) } }))
    const scene = sceneOf({ units: [{ key: K(810), edges: sides }], regions: [] })
    const ref = resolveSelectableRef(solidSnap(OBJECT), rayAt(0.5, 0.5 * Math.sqrt(3)), deps(scene, [])) as NodeRef
    expect(ref).toEqual(draft('draft-curve', GROUND, K(0xb01)))
    expect(edgeKeysOf(scene as unknown as PlaneUnitsScene, ref)).toHaveLength(6)
  })
})

describe('refineToDraft respects the editing-context scope', () => {
  const OBJECT = 7n
  const rayAt = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
  const eligible7 = (o: bigint) => o === 7n
  const sceneOf = (store: FStore) =>
    fakeScene({
      faceZ: 0,
      face: { object: OBJECT, depth: 5, face: 15n },
      drawn: { 0: { units: [{ key: K(901), edges: [{ key: K(20), a: [0, 0, 0], b: [1, 0, 0], store }] }], regions: [] } },
    })

  it('inside a context a free sketch coplanar with the struck face is NOT selectable; the object stands', () => {
    const ref = resolveSelectableRef(solidSnap(OBJECT), rayAt(0.5, 0), deps(sceneOf({ kind: 'free', sketch: 21 }), GROUP_CTX, 100, WPP, eligible7))
    expect(ref).toMatchObject({ kind: 'object', id: OBJECT })
  })

  it('inside a context the drawing of an eligible owner IS selectable', () => {
    const ref = resolveSelectableRef(solidSnap(OBJECT), rayAt(0.5, 0), deps(sceneOf({ kind: 'topology', object: 7 }), GROUP_CTX, 100, WPP, eligible7))
    expect(ref).toEqual(draft('draft-edge', GROUND, K(20)))
  })

  it('inside a context the drawing of an ineligible owner is not selectable', () => {
    const ref = resolveSelectableRef(solidSnap(OBJECT), rayAt(0.5, 0), deps(sceneOf({ kind: 'topology', object: 7 }), GROUP_CTX, 100, WPP, () => false))
    expect(ref).toMatchObject({ kind: 'object', id: OBJECT })
  })

  it('at top level the same free sketch IS selectable', () => {
    const ref = resolveSelectableRef(solidSnap(OBJECT), rayAt(0.5, 0), deps(sceneOf({ kind: 'free', sketch: 21 }), []))
    expect(ref).toEqual(draft('draft-edge', GROUND, K(20)))
  })
})

describe('resolveSelectableRef — drawing refinement on a struck face (docs/design/unified-drafting.md §8)', () => {
  const OBJECT = 7n
  const topo: FStore = { kind: 'topology', object: 7 }
  const rayAt = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
  /** A chord drawn across the face: one topology line (0,0)-(1,0). */
  const chordListing = (z: number, to = 1): Listing => ({
    units: [{ key: K(901), edges: [{ key: K(20), a: [0, 0, z], b: [to, 0, z], store: topo }] }],
    regions: [],
  })
  const faceScene = (drawn: Listing | null, faceZ = 0, extra: Partial<SceneStubs> = {}) =>
    fakeScene({
      faceZ,
      face: { object: OBJECT, depth: 5, face: 15n },
      ...(drawn ? { drawn: { [faceZ]: drawn } } : {}),
      ...extra,
    })

  it('a plain-object pick whose struck face carries a drawn shape resolves to the shape, in the face-plane context', () => {
    const listing = rectListing(0.5, { store: topo, region: 6 })
    listing.regions[0].over = { object: 7, face: 15 }
    const ref = resolveSelectableRef(solidSnap(OBJECT), DOWN, deps(faceScene(listing, 0.5), []))
    expect(ref).toEqual(draft('draft-island', '{"plane":[0,0,0.5,0,0,1]}', K(900)))
  })

  it('a pick whose struck face has no drawing resolves to the plain object', () => {
    const ref = resolveSelectableRef(solidSnap(OBJECT), DOWN, deps(faceScene(null), []))
    expect(ref).toMatchObject({ kind: 'object', id: OBJECT })
  })

  it('a click on bare face AROUND a drawn shape resolves the object; inside the shape resolves the shape', () => {
    // The face's own region (bounded only by the solid's edges) is
    // background, not a shape.
    const listing = rectListing(0, { store: topo, region: 6 })
    listing.regions.push({ key: K(7), outer: rectRing(0, 10), holes: [rectRing(0, 1)], over: { object: 7, face: 15 } })
    const around = resolveSelectableRef(solidSnap(OBJECT), rayAt(5, 0), deps(faceScene(listing), []))
    expect(around).toMatchObject({ kind: 'object', id: OBJECT })
    const inside = resolveSelectableRef(solidSnap(OBJECT), rayAt(0, 0), deps(faceScene(listing), []))
    expect(inside).toEqual(draft('draft-island', GROUND, K(900)))
  })

  it('a ground sketch resolves a draft ref on the ground context', () => {
    const scene = fakeScene({ drawn: { 0: rectListing(0, { unit: 910 }) }, region: { sketch: 21n, region: 5n } })
    const ref = resolveSelectableRef(regionSnap(21n, 5n), DOWN, deps(scene, []))
    expect(ref).toEqual(draft('draft-island', ctxGround(), K(910)))
  })

  it('a mixed-store island (topology + free edges) resolves whole as ONE island, from fill clicks on a face or a sketch', () => {
    const edges = rectEdges(0, 1, 100)
    edges[0].store = topo
    edges[1].store = topo
    edges[2].store = { kind: 'free', sketch: 21 }
    edges[3].store = { kind: 'free', sketch: 21 }
    const listing: Listing = { units: [{ key: K(900), edges }], regions: [{ key: K(5), outer: rectRing(0, 1) }] }
    const expected = draft('draft-island', GROUND, K(900))
    // From the object's face:
    expect(resolveSelectableRef(solidSnap(OBJECT), DOWN, deps(faceScene(listing), []))).toEqual(expected)
    // From the free sketch's region:
    const sk = fakeScene({ drawn: { 0: listing }, region: { sketch: 21n, region: 5n } })
    expect(resolveSelectableRef(regionSnap(21n, 5n), DOWN, deps(sk, []))).toEqual(expected)
    // A click on a topology edge or a free edge of it names that edge, not
    // half an island.
    expect(resolveSelectableRef(solidSnap(OBJECT), rayAt(0, -1), deps(faceScene(listing), [])))
      .toEqual(draft('draft-edge', GROUND, K(100)))
    expect(resolveSelectableRef(solidSnap(OBJECT), rayAt(0, 1), deps(faceScene(listing), [])))
      .toEqual(draft('draft-edge', GROUND, K(102)))
  })

  it('a mixed island found by the instance scoped probe selects whole, eligible owner or not', () => {
    const edges = rectEdges(0, 1, 100)
    edges[0].store = { kind: 'owned', object: 42, sketch: 31 }
    edges[1].store = { kind: 'free', sketch: 21 }
    const listing: Listing = { units: [{ key: K(900), edges }], regions: [{ key: K(5), outer: rectRing(0, 1) }] }
    const mk = () => fakeScene({ drawn: { 0: listing }, instanceRegion: { sketch: 31n, region: 5n }, face: { object: 42n, instance: 7n, depth: 8 } })
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(mk(), INSTANCE_CTX, 100, WPP, eligible42)))
      .toEqual(draft('draft-island', INSTANCE_PLANE, K(900)))
    expect(resolveSelectableRef(bareSnap(), DOWN, deps(mk(), INSTANCE_CTX, 100, WPP, () => false)))
      .toEqual(draft('draft-island', INSTANCE_PLANE, K(900)))
  })

  it('a group-wrapped object never refines to a drawing, even when the struck face carries one', () => {
    // Simulate the Viewport's resolveObject wrapping the object in its
    // containing group — only a pick that resolved to the OBJECT itself
    // (kind 'object', same id as the hit) is refined.
    const scene = faceScene(rectListing(0, { store: topo }))
    const wrapped: ResolveDeps = { ...deps(scene, []), resolveObject: () => ({ kind: 'group', id: 3n }) }
    expect(resolveSelectableRef(solidSnap(OBJECT), DOWN, wrapped)).toEqual({ kind: 'group', id: 3n })
  })

  it('an edge snap onto a solid edge resolves the drawn line under the ray by geometry, not by the snapped edge handle', () => {
    // The snap names a solid edge handle (99) the drawing knows nothing
    // about; the ray's plane pick decides what is selected.
    const scene = faceScene(chordListing(0))
    const snap: Snap = { ...base, kind: 'on-edge', elementKind: 'edge', object: OBJECT, element: 99n }
    expect(resolveSelectableRef(snap, DOWN, deps(scene, []))).toEqual(draft('draft-edge', GROUND, K(20)))
  })

  it('a snap within 8 px of a drawn line resolves to it without an exact edge snap', () => {
    const log: { ctx: string; tol?: number }[] = []
    const scene = faceScene(chordListing(0), 0, { log })
    // 1.5 mm off the line at 0.25 mm per pixel: 6 px.
    const snap: Snap = { ...base, kind: 'on-face', object: OBJECT, x: 0.5, y: 0.0015, z: 0 }
    expect(resolveSelectableRef(snap, rayAt(0.5, 0.0015), deps(scene, []))).toEqual(draft('draft-edge', GROUND, K(20)))
    // The tolerance handed to the kernel is exactly 8 screen pixels.
    const picks = log.filter((l) => l.tol !== undefined)
    expect(picks).toHaveLength(1)
    expect(picks[0].tol).toBeCloseTo(8 * WPP, 12)
  })

  it('a snap farther than 8 px from every drawn line does not refine', () => {
    const scene = faceScene(chordListing(0))
    const snap: Snap = { ...base, kind: 'on-face', object: OBJECT, x: 0.5, y: 0.5, z: 0 }
    expect(resolveSelectableRef(snap, rayAt(0.5, 0.5), deps(scene, []))).toMatchObject({ kind: 'object', id: OBJECT })
  })

  it('the pick radius is a screen distance: zoomed in on a millimetre part it shrinks with the view', () => {
    // A 1.9 mm line: at 0.01 mm per pixel the radius is 0.08 mm, so a click
    // 0.5 mm off it (50 px) misses — where a fixed 2 mm world radius would
    // have grabbed it — and a click 0.05 mm off (5 px) still selects it.
    const log: { ctx: string; tol?: number }[] = []
    const scene = faceScene(chordListing(0, 0.0019), 0, { log })
    const zoomedIn = 1e-5
    const far: Snap = { ...base, kind: 'on-face', object: OBJECT, x: 0.001, y: 0.0005, z: 0 }
    expect(resolveSelectableRef(far, rayAt(0.001, 0.0005), deps(scene, [], 100, zoomedIn))).toMatchObject({
      kind: 'object',
      id: OBJECT,
    })
    const near: Snap = { ...base, kind: 'on-face', object: OBJECT, x: 0.001, y: 0.00005, z: 0 }
    expect(resolveSelectableRef(near, rayAt(0.001, 0.00005), deps(scene, [], 100, zoomedIn))).toEqual(
      draft('draft-edge', GROUND, K(20)),
    )
    for (const l of log.filter((x) => x.tol !== undefined)) expect(l.tol).toBeCloseTo(8 * zoomedIn, 14)
  })

  it('a click on a guide lying over a drawn line selects the line, tested at the face hit under the cursor', () => {
    // The snap is a guide cue (no object): the pick runs along the ray, where
    // it meets the face (0.5, 0.0015, 0) — 1.5 mm, 6 px off the line — not at
    // the guide's own snap point far away.
    const scene = faceScene(chordListing(0))
    const onGuide: Snap = { ...base, kind: 'on-guide', x: 0.5, y: 0.3, z: 0 }
    // A non-unit direction: the pick depth is a distance along the ray.
    const ray: Ray = { origin: [0.5, 0.0015, 5], direction: [0, 0, -2] }
    expect(resolveSelectableRef(onGuide, ray, deps(scene, []))).toEqual(draft('draft-edge', GROUND, K(20)))
    // And a guide click whose face hit is far from every line still selects
    // the solid.
    const far: Ray = { origin: [0.5, 0.3, 5], direction: [0, 0, -1] }
    expect(resolveSelectableRef(onGuide, far, deps(scene, []))).toMatchObject({ kind: 'object', id: OBJECT })
  })

  it('a guide snap held on a line drawn on a face selects the line from beyond the pick radius', () => {
    const scene = faceScene(chordListing(0))
    const onGuide: Snap = { ...base, kind: 'on-guide', x: 0.5, y: 0, z: 0 }
    const ray: Ray = { origin: [0.5, 0.0075, 5], direction: [0, 0, -1] } // 30 px off the line
    expect(resolveSelectableRef(onGuide, ray, deps(scene, []))).toEqual(draft('draft-edge', GROUND, K(20)))
  })

  it('the pick asks the camera for its scale at the snapped point', () => {
    const scene = faceScene(chordListing(0))
    const asked: (readonly [number, number, number])[] = []
    const d: ResolveDeps = {
      ...deps(scene, []),
      worldPerPixelAt: (p) => {
        asked.push(p)
        return WPP
      },
    }
    const snap: Snap = { ...base, kind: 'on-face', object: OBJECT, x: 0.5, y: 0.0015, z: 0 }
    expect(resolveSelectableRef(snap, rayAt(0.5, 0.0015), d)).toEqual(draft('draft-edge', GROUND, K(20)))
    expect(asked).toEqual([[0.5, 0.0015, 0]])
  })

  it('nothing refines when the scene double has no plane listing at all', () => {
    const scene = faceScene(chordListing(0), 0, { noPlanes: true })
    const ref = resolveSelectableRef(solidSnap(OBJECT), DOWN, deps(scene, []))
    expect(ref).toMatchObject({ kind: 'object', id: OBJECT })
  })
})
