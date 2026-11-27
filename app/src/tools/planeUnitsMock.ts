/**
 * Test double for the wasm plane-unit queries (`plane_units_json`,
 * `pick_plane_unit`), serving hand-written listings per context. The pick is
 * real geometry over the listing — the ray meets the context's plane, the
 * nearest edge within the tolerance wins, else the region holding the point
 * (holes honoured) — so tool tests drive real rays and the tool resolves the
 * same region it would against the kernel.
 */
import { vi } from 'vitest'
import { planeOfCtx } from './planeUnits'
import { offsetRegionLoops } from './offsetMath'

export const hexKey = (n: number): string => n.toString(16).padStart(16, '0')

type P = readonly [number, number, number]

export interface MockRegion {
  key: number
  outer: P[]
  holes?: P[][]
  over?: { object: number; face: number | null; why?: string } | null
  extrude?: { sketch: number; region: number } | null
}

export interface MockEdge {
  key: number
  a: P
  b: P
  store?: Record<string, unknown>
  curve?: { center: P; radius: number | null } | null
}

export interface MockUnit {
  key: number
  edges: MockEdge[]
  curves?: Array<{ key: number; curveKey: number; kind?: 'circle' | 'polygon'; center: P; radius: number | null; edges: number[] }>
}

export interface MockListing {
  units?: MockUnit[]
  regions?: MockRegion[]
}

const FREE = { kind: 'free', sketch: 1 }

/** The edges of a closed ring as one unit of `store` edges (keys start at `firstKey`). */
export function ringUnit(unitKey: number, firstKey: number, ring: P[], store: Record<string, unknown> = FREE): MockUnit {
  return {
    key: unitKey,
    edges: ring.map((a, i) => ({ key: firstKey + i, a, b: ring[(i + 1) % ring.length], store })),
  }
}

function listingJson(l: MockListing): string {
  return JSON.stringify({
    units: (l.units ?? []).map((u) => ({
      key: hexKey(u.key),
      edges: u.edges.map((e) => ({
        key: hexKey(e.key),
        a: e.a,
        b: e.b,
        store: e.store ?? FREE,
        curve: e.curve ?? null,
      })),
      curves: (u.curves ?? []).map((c) => ({
        key: hexKey(c.key),
        curve_key: hexKey(c.curveKey),
        kind: c.kind ?? 'circle',
        center: c.center,
        radius: c.radius,
        edges: c.edges.map(hexKey),
      })),
    })),
    regions: (l.regions ?? []).map((r) => ({
      key: hexKey(r.key),
      outer: r.outer,
      holes: r.holes ?? [],
      over: r.over ?? null,
      extrude: r.extrude ?? null,
    })),
  })
}

function inRing(p: P, ring: readonly P[], n: P): boolean {
  const ax = Math.abs(n[0]) >= Math.abs(n[1]) && Math.abs(n[0]) >= Math.abs(n[2]) ? 0 : Math.abs(n[1]) >= Math.abs(n[2]) ? 1 : 2
  const [u, v] = ax === 0 ? [1, 2] : ax === 1 ? [0, 2] : [0, 1]
  let inside = false
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const a = ring[i], b = ring[j]
    if (a[v] > p[v] !== b[v] > p[v] && p[u] < ((b[u] - a[u]) * (p[v] - a[v])) / (b[v] - a[v]) + a[u]) inside = !inside
  }
  return inside
}

function segDist(p: P, a: P, b: P): number {
  const d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
  const l2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
  const t = l2 > 0 ? Math.max(0, Math.min(1, ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1] + (p[2] - a[2]) * d[2]) / l2)) : 0
  return Math.hypot(p[0] - a[0] - d[0] * t, p[1] - a[1] - d[1] * t, p[2] - a[2] - d[2] * t)
}

/** `{ plane_units_json, pick_plane_unit }` over `listings` (keyed by the
 *  canonical context string). An unlisted context is an empty plane. */
export function planeUnitsMock(listings: Record<string, MockListing>) {
  const pick = (ctx: string, ray: ArrayLike<number>, tol: number): string => {
    const pl = planeOfCtx(ctx)
    const l = listings[ctx]
    if (pl === null || l === undefined) return 'null'
    const den = pl.normal[0] * ray[3] + pl.normal[1] * ray[4] + pl.normal[2] * ray[5]
    if (Math.abs(den) < 1e-12) return 'null'
    const t = ((pl.point[0] - ray[0]) * pl.normal[0] + (pl.point[1] - ray[1]) * pl.normal[1] + (pl.point[2] - ray[2]) * pl.normal[2]) / den
    if (t < 0) return 'null'
    const hit: P = [ray[0] + ray[3] * t, ray[1] + ray[4] * t, ray[2] + ray[5] * t]
    let best: { d: number; unit: number; edge: number } | null = null
    for (const u of l.units ?? []) for (const e of u.edges) {
      const d = segDist(hit, e.a, e.b)
      if (d <= tol && (best === null || d < best.d)) best = { d, unit: u.key, edge: e.key }
    }
    if (best !== null) return JSON.stringify({ edge: hexKey(best.edge), unit: hexKey(best.unit), curve: null, run: null })
    // Smallest containing region wins (nested regions resolve to the innermost).
    let inner: { key: number; area: number } | null = null
    for (const r of l.regions ?? []) {
      if (!inRing(hit, r.outer, pl.normal as unknown as P) || (r.holes ?? []).some((h) => inRing(hit, h, pl.normal as unknown as P))) continue
      let area = 0
      for (let i = 0; i < r.outer.length; i++) {
        const a = r.outer[i], b = r.outer[(i + 1) % r.outer.length]
        area += a[0] * b[1] - a[1] * b[0] + a[1] * b[2] - a[2] * b[1] + a[2] * b[0] - a[0] * b[2]
      }
      area = Math.abs(area)
      if (inner === null || area < inner.area) inner = { key: r.key, area }
    }
    return inner === null ? 'null' : JSON.stringify({ region: hexKey(inner.key) })
  }
  // The kernel's offset preview, stood in for by the mitred TS offset in the
  // same `[loopCount, n0, xyz.., n1, ..]` wire format; a collapse throws as
  // the kernel's typed error would.
  const offsetPreview = (ctx: string, key: bigint, distance: number): Float64Array => {
    const pl = planeOfCtx(ctx)
    const r = (listings[ctx]?.regions ?? []).find((x) => BigInt(x.key) === key)
    if (pl === null || r === undefined) throw new Error('UnknownRegion: no such region')
    const loops = offsetRegionLoops(
      r.outer.map((q) => [q[0], q[1], q[2]] as [number, number, number]),
      (r.holes ?? []).map((h) => h.map((q) => [q[0], q[1], q[2]] as [number, number, number])),
      pl.normal,
      distance,
    )
    if (loops === null) throw new Error('OffsetCollapsed: the offset collapses the region')
    const out: number[] = [loops.length]
    for (const loop of loops) out.push(loop.length / 3, ...loop)
    return new Float64Array(out)
  }
  return {
    plane_units_json: vi.fn((ctx: string) => listingJson(listings[ctx] ?? {})),
    pick_plane_unit: vi.fn((ctx: string, ray: Float64Array, tol: number) => pick(ctx, ray, tol)),
    offset_region_preview: vi.fn(offsetPreview),
  }
}
