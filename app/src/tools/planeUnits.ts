/**
 * planeUnits — the app's view of "the drawing on a plane"
 * (docs/design/unified-drafting.md §0, §4.5, §8).
 *
 * The PLANE is the drafting context. Everything drawn on one plane — the face
 * topology of the solids lying on it (imprints), object-owned sketches, free
 * sketches — is one set of drawing UNITS, whichever store each edge lives in.
 * A unit (an island) can mix stores. This module parses the kernel's
 * `plane_units_json(ctx)` listing, caches it per context, and answers the
 * questions every consumer asks of a selection ref: which edge keys does it
 * stand for, what does it look like, which stores and objects does it touch.
 *
 * ## The context string (PlaneCtx)
 *
 * Always the canonical plane form `{"plane":[px,py,pz,nx,ny,nz]}`, plus
 * `"instance":<id>` inside an entered component instance. The point is the
 * plane's closest point to the world origin, the normal is a unit vector, and
 * both are rounded to 1e-12 with `-0` folded to `0`, so one plane reached by
 * any route (a face, a sketch, the ground) yields ONE string. Faces use their
 * outward normal, the ground +Z. The string is both the kernel input and the
 * cache/identity key of every ref.
 *
 * ## Cache
 *
 * Listings are cached per context string and keyed to the document's
 * `history_generation` (changes on every committed mutation, undo, and redo),
 * so a document change invalidates them without any hook. `invalidatePlaneUnits`
 * drops everything for the events that change what is listed without moving
 * history (hide/unhide, a scene refresh).
 */

import { t } from '@lingui/core/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef, NodeKind } from '../panels/treeModel'
import type { V3 } from '../viewport/geoHelpers'
import { applyAffine3x4 } from '../viewport/geoHelpers'
import { worldFacePlane } from './faceDraw'
import { isRectangle } from './imprints'

// ── Context strings ─────────────────────────────────────────────────────────

const ROUND = 1e12

function r12(x: number): number {
  const v = Math.round(x * ROUND) / ROUND
  return v === 0 ? 0 : v // fold -0
}

/** A plane in the canonical form `[px,py,pz,nx,ny,nz]`: unit normal, the
 *  point the plane's closest to the world origin, everything rounded. `null`
 *  for a zero or non-finite normal. */
export function canonicalPlane(point: readonly number[], normal: readonly number[]): [number, number, number, number, number, number] | null {
  const len = Math.hypot(normal[0], normal[1], normal[2])
  if (!(len > 0) || !Number.isFinite(len)) return null
  const nx = r12(normal[0] / len)
  const ny = r12(normal[1] / len)
  const nz = r12(normal[2] / len)
  const d = point[0] * nx + point[1] * ny + point[2] * nz
  return [r12(nx * d), r12(ny * d), r12(nz * d), nx, ny, nz]
}

/** The context string of the plane through `point` with `normal`, optionally
 *  inside the entered instance. `null` for a degenerate normal. */
export function ctxOfPlane(point: readonly number[], normal: readonly number[], instance: bigint | null = null): string | null {
  const p = canonicalPlane(point, normal)
  if (p === null) return null
  return instance !== null
    ? JSON.stringify({ plane: p, instance: Number(instance) })
    : JSON.stringify({ plane: p })
}

/** The ground plane's context (z = 0, +Z). */
export function ctxGround(): string {
  return ctxOfPlane([0, 0, 0], [0, 0, 1]) as string
}

/** The context of a solid's face — its WORLD plane with the outward normal. */
export function ctxOfFace(
  scene: WasmScene,
  object: bigint,
  face: bigint,
  instance: bigint | null = null,
): string | null {
  const wp = worldFacePlane(scene, object, face, instance)
  return wp === null ? null : ctxOfPlane(wp.point, wp.normal, instance)
}

/** The context of a sketch's plane (posed to world through `instance` for a
 *  definition-owned sketch). `null` for a stale sketch. */
export function ctxOfSketch(scene: WasmScene, sketch: bigint, instance: bigint | null = null): string | null {
  const sp = scene.sketch_plane(sketch)
  if (sp === undefined) return null
  let point: V3 = [sp[0], sp[1], sp[2]]
  let normal: V3 = [sp[3], sp[4], sp[5]]
  if (instance !== null) {
    const pose = scene.instance_pose(instance)
    if (pose === undefined) return null
    point = applyAffine3x4(pose, point)
    normal = [
      pose[0] * sp[3] + pose[1] * sp[4] + pose[2] * sp[5],
      pose[4] * sp[3] + pose[5] * sp[4] + pose[6] * sp[5],
      pose[8] * sp[3] + pose[9] * sp[4] + pose[10] * sp[5],
    ]
  }
  return ctxOfPlane(point, normal, instance)
}

/** The context of ONE detached sketch (separate-shapes design D5): that
 *  sketch's own drafting context, in which only its drawing takes part. The
 *  kernel hands this string back as a pick's `ctx`; building it here keeps
 *  every ref of the sketch on one canonical string. */
export function ctxOfSketchOnly(sketch: bigint | number, instance: bigint | number | null = null): string {
  return instance !== null
    ? JSON.stringify({ sketch: Number(sketch), instance: Number(instance) })
    : JSON.stringify({ sketch: Number(sketch) })
}

/** The sketch (and instance) a sketch-only context names, or `null` for any
 *  other context (`ctxOfSketchOnly`'s inverse). */
export function sketchOfCtx(ctx: string): { sketch: bigint; instance: bigint | null } | null {
  try {
    const v = JSON.parse(ctx) as { sketch?: unknown; instance?: unknown }
    if (typeof v.sketch !== 'number') return null
    return { sketch: BigInt(v.sketch), instance: typeof v.instance === 'number' ? BigInt(v.instance) : null }
  } catch {
    return null
  }
}

/** The plane of a context string, or `null` if it is not a plane context. */
export function planeOfCtx(ctx: string): { point: V3; normal: V3; instance: bigint | null } | null {
  try {
    const v = JSON.parse(ctx) as { plane?: number[]; instance?: number }
    if (!Array.isArray(v.plane) || v.plane.length !== 6) return null
    return {
      point: [v.plane[0], v.plane[1], v.plane[2]],
      normal: [v.plane[3], v.plane[4], v.plane[5]],
      instance: typeof v.instance === 'number' ? BigInt(v.instance) : null,
    }
  } catch {
    return null
  }
}

/** `planeOfCtx`, also answering a detached sketch's own context from the
 *  sketch's plane (a world sketch; one inside an instance is not posed here,
 *  so it answers `null`). */
export function planeOfCtxIn(
  scene: { sketch_plane?(sketch: bigint): Float64Array | undefined },
  ctx: string,
): { point: V3; normal: V3; instance: bigint | null } | null {
  const pl = planeOfCtx(ctx)
  if (pl !== null) return pl
  const of = sketchOfCtx(ctx)
  if (of === null || of.instance !== null || scene.sketch_plane === undefined) return null
  const sp = scene.sketch_plane(of.sketch)
  return sp === undefined ? null : { point: [sp[0], sp[1], sp[2]], normal: [sp[3], sp[4], sp[5]], instance: null }
}

/** The context of `ctx`'s plane moved through `affine` (3x4 row-major, world):
 *  the plane a whole-plane out-of-plane transform lands on. */
export function ctxThroughAffine(ctx: string, affine: ArrayLike<number>): string | null {
  const pl = planeOfCtx(ctx)
  if (pl === null) return null
  const p = applyAffine3x4(affine as unknown as Parameters<typeof applyAffine3x4>[0], pl.point)
  // A normal maps by the inverse transpose (so a slanted plane stays the
  // plane of its moved points under a non-uniform scale): with the linear
  // part's columns c0..c2, L^-T·n = (n.x·c1×c2 + n.y·c2×c0 + n.z·c0×c1) / det.
  const col = (c: number): V3 => [affine[c], affine[4 + c], affine[8 + c]]
  const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
  const [c0, c1, c2] = [col(0), col(1), col(2)]
  const k0 = cross(c1, c2)
  const k1 = cross(c2, c0)
  const k2 = cross(c0, c1)
  const det = c0[0] * k0[0] + c0[1] * k0[1] + c0[2] * k0[2]
  if (det === 0) return null
  const [x, y, z] = pl.normal
  const s = Math.sign(det)
  const n: V3 = [
    s * (x * k0[0] + y * k1[0] + z * k2[0]),
    s * (x * k0[1] + y * k1[1] + z * k2[1]),
    s * (x * k0[2] + y * k1[2] + z * k2[2]),
  ]
  return ctxOfPlane(p, n, pl.instance)
}

// ── Keys ────────────────────────────────────────────────────────────────────

/** A 16-hex kernel key as a bigint. */
export function keyToId(key: string): bigint {
  return BigInt('0x' + key)
}

/** The wasm `BigUint64Array` of these hex keys. */
export function keysToArray(keys: readonly string[]): BigUint64Array {
  return new BigUint64Array(keys.map(keyToId))
}

// ── Parsed listing ──────────────────────────────────────────────────────────

export type DraftStore =
  | { kind: 'topology'; object: bigint }
  | { kind: 'owned'; object: bigint; sketch: bigint }
  /** `detached`: the sketch is a detached shape — in no plane's arrangement,
   *  its own drafting context (separate-shapes design D5). */
  | { kind: 'free'; sketch: bigint; detached?: true }

export interface PlaneEdge {
  key: string
  a: V3
  b: V3
  store: DraftStore
  curve: { center: V3; radius: number | null } | null
}

/** A CURVE RUN: the piece of one circle, arc, or regular polygon a click
 *  selects — its facets reachable through points nothing else uses. A circle
 *  other drawing crosses is several runs sharing one `curveKey`. */
export interface PlaneCurveRun {
  key: string
  curveKey: string
  kind: 'circle' | 'polygon'
  center: V3
  /** `null` for a circle inside a non-uniformly scaled instance (an ellipse). */
  radius: number | null
  edges: string[]
}

export interface PlaneUnit {
  key: string
  edges: PlaneEdge[]
  curves: PlaneCurveRun[]
}

export interface PlaneRegion {
  key: string
  outer: V3[]
  holes: V3[][]
  over: { object: bigint; face: bigint | null; why?: 'sliver' | 'other' } | null
  extrude: { sketch: bigint; region: bigint } | null
}

export interface PlaneUnits {
  ctx: string
  units: PlaneUnit[]
  regions: PlaneRegion[]
  unitByKey: Map<string, PlaneUnit>
  edgeByKey: Map<string, { unit: PlaneUnit; edge: PlaneEdge }>
  runByKey: Map<string, { unit: PlaneUnit; run: PlaneCurveRun }>
  /** The run each edge belongs to (circle, arc, or polygon edges only). */
  runOfEdge: Map<string, PlaneCurveRun>
  regionByKey: Map<string, PlaneRegion>
}

/** The slice of the wasm `Scene` the listing needs. */
export interface PlaneUnitsScene {
  plane_units_json(ctx: string): string
  /** A sketch's plane (a detached sketch's own context has no plane string). */
  sketch_plane?(sketch: bigint): Float64Array | undefined
  pick_plane_unit?(ctx: string, ray: Float64Array, tolerance: number): string
  history_generation?(): bigint
}

function v3(x: unknown): V3 {
  const a = x as number[]
  return [Number(a[0]), Number(a[1]), Number(a[2])]
}

function parseStore(s: Record<string, unknown>): DraftStore {
  if (s.kind === 'topology') return { kind: 'topology', object: BigInt(s.object as number) }
  if (s.kind === 'owned') return { kind: 'owned', object: BigInt(s.object as number), sketch: BigInt(s.sketch as number) }
  const sketch = BigInt(s.sketch as number)
  return s.detached === true ? { kind: 'free', sketch, detached: true } : { kind: 'free', sketch }
}

/** Parse `plane_units_json`. Malformed entries are skipped, never thrown on —
 *  a render or pick must not die on one odd entry. */
export function parsePlaneUnits(ctx: string, json: string): PlaneUnits {
  const out: PlaneUnits = {
    ctx,
    units: [],
    regions: [],
    unitByKey: new Map(),
    edgeByKey: new Map(),
    runByKey: new Map(),
    runOfEdge: new Map(),
    regionByKey: new Map(),
  }
  let raw: { units?: unknown[]; regions?: unknown[] }
  try {
    raw = JSON.parse(json) as { units?: unknown[]; regions?: unknown[] }
  } catch {
    return out
  }
  for (const u of raw.units ?? []) {
    const ur = u as { key?: string; edges?: unknown[] }
    if (typeof ur.key !== 'string' || !Array.isArray(ur.edges)) continue
    const unit: PlaneUnit = { key: ur.key, edges: [], curves: [] }
    for (const e of ur.edges) {
      const er = e as Record<string, unknown>
      if (typeof er.key !== 'string' || typeof er.store !== 'object' || er.store === null) continue
      const c = er.curve as { center?: unknown; radius?: unknown } | null
      const edge: PlaneEdge = {
        key: er.key,
        a: v3(er.a),
        b: v3(er.b),
        store: parseStore(er.store as Record<string, unknown>),
        curve: c !== null && c !== undefined && c.center !== undefined
          ? { center: v3(c.center), radius: c.radius === null || c.radius === undefined ? null : Number(c.radius) }
          : null,
      }
      unit.edges.push(edge)
      out.edgeByKey.set(edge.key, { unit, edge })
    }
    for (const c of (ur as { curves?: unknown[] }).curves ?? []) {
      const cr = c as Record<string, unknown>
      if (typeof cr.key !== 'string' || typeof cr.curve_key !== 'string' || !Array.isArray(cr.edges)) continue
      const run: PlaneCurveRun = {
        key: cr.key,
        curveKey: cr.curve_key,
        kind: cr.kind === 'polygon' ? 'polygon' : 'circle',
        center: v3(cr.center),
        radius: cr.radius === null || cr.radius === undefined ? null : Number(cr.radius),
        edges: (cr.edges as unknown[]).filter((k): k is string => typeof k === 'string'),
      }
      unit.curves.push(run)
      out.runByKey.set(run.key, { unit, run })
      for (const k of run.edges) out.runOfEdge.set(k, run)
    }
    out.units.push(unit)
    out.unitByKey.set(unit.key, unit)
  }
  for (const r of raw.regions ?? []) {
    const rr = r as Record<string, unknown>
    if (typeof rr.key !== 'string') continue
    const over = rr.over as { object: number; face: number | null; why?: string } | null
    const ex = rr.extrude as { sketch: number; region: number } | null
    const region: PlaneRegion = {
      key: rr.key,
      outer: ((rr.outer as unknown[]) ?? []).map(v3),
      holes: ((rr.holes as unknown[][]) ?? []).map((h) => h.map(v3)),
      over: over !== null && over !== undefined
        ? {
          object: BigInt(over.object),
          face: over.face === null || over.face === undefined ? null : BigInt(over.face),
          ...(over.why === 'sliver' || over.why === 'other' ? { why: over.why } : {}),
        }
        : null,
      extrude: ex !== null && ex !== undefined
        ? { sketch: BigInt(ex.sketch), region: BigInt(ex.region) }
        : null,
    }
    out.regions.push(region)
    out.regionByKey.set(region.key, region)
  }
  return out
}

// ── Cache ───────────────────────────────────────────────────────────────────

interface CacheEntry {
  scene: PlaneUnitsScene
  gen: string | null
  byCtx: Map<string, PlaneUnits | null>
}
let cache: CacheEntry | null = null

/** Drop every cached listing (the document changed in a way history does not
 *  show: a hide/unhide, a scene refresh). */
export function invalidatePlaneUnits(): void {
  cache = null
  allCache = null
}

/** Push the hidden set to the kernel's inference state and drop the cached
 *  listings: hiding changes what a listing answers without moving history, so
 *  every `set_hidden` goes through here. */
export function setKernelHidden(
  scene: { set_hidden(objects: BigUint64Array, instances: BigUint64Array): void },
  objectIds: readonly bigint[],
  instanceIds: readonly bigint[],
): void {
  scene.set_hidden(new BigUint64Array(objectIds), new BigUint64Array(instanceIds))
  invalidatePlaneUnits()
}

function genOf(scene: PlaneUnitsScene): string | null {
  try {
    return scene.history_generation !== undefined ? scene.history_generation().toString() : null
  } catch {
    return null
  }
}

/** The drawing on `ctx`'s plane, cached until the document changes. `null`
 *  when the context is stale or the kernel refuses the listing. */
/** Puts one plane's listing, read some other way (the whole document's in
 *  one call), where `readPlaneUnits` finds it for this document revision. */
export function seedPlaneUnits(scene: PlaneUnitsScene, ctx: string, pu: PlaneUnits | null): void {
  const gen = genOf(scene)
  if (gen === null) return
  if (cache === null || cache.scene !== scene || cache.gen !== gen) {
    cache = { scene, gen, byCtx: new Map() }
  }
  cache.byCtx.set(ctx, pu)
}

export function readPlaneUnits(scene: PlaneUnitsScene, ctx: string): PlaneUnits | null {
  const gen = genOf(scene)
  if (cache === null || cache.scene !== scene || cache.gen !== gen || gen === null) {
    cache = { scene, gen, byCtx: new Map() }
  }
  if (gen !== null && cache.byCtx.has(ctx)) return cache.byCtx.get(ctx) ?? null
  let pu: PlaneUnits | null
  try {
    pu = parsePlaneUnits(ctx, scene.plane_units_json(ctx))
  } catch {
    pu = null
  }
  if (gen !== null) cache.byCtx.set(ctx, pu)
  return pu
}

// ── Curves ──────────────────────────────────────────────────────────────────

/** The runs of the circle (or polygon) with `curveKey` across the listing —
 *  one for a whole circle, several when other drawing crosses it. */
export function runsOfCurve(pu: PlaneUnits, curveKey: string): PlaneCurveRun[] {
  const out: PlaneCurveRun[] = []
  for (const u of pu.units) for (const r of u.curves) if (r.curveKey === curveKey) out.push(r)
  return out
}

/** The edges a `draft-curve` key names: a RUN's edges, or — for a circle's
 *  own curve key, what a double-click widens to — every run's edges. */
export function edgesOfCurveRef(pu: PlaneUnits, key: string): PlaneEdge[] {
  const run = pu.runByKey.get(key)
  const runs = run !== undefined ? [run.run] : runsOfCurve(pu, key)
  const out: PlaneEdge[] = []
  for (const r of runs) {
    for (const k of r.edges) {
      const e = pu.edgeByKey.get(k)
      if (e !== undefined) out.push(e.edge)
    }
  }
  return out
}

// ── Picking ─────────────────────────────────────────────────────────────────

export type PlanePick =
  | { edge: string; unit: string; curve: string | null; run: string | null; ctx?: string }
  | { region: string; ctx?: string }

/** The kernel's pick in `ctx`'s plane (see `pick_plane_unit`). */
export function pickPlaneUnit(
  scene: PlaneUnitsScene,
  ctx: string,
  ray: Float64Array,
  tolerance: number,
): PlanePick | null {
  if (scene.pick_plane_unit === undefined) return null
  let json: string
  try {
    json = scene.pick_plane_unit(ctx, ray, tolerance)
  } catch {
    return null
  }
  let v: {
    edge?: string
    unit?: string
    curve?: string | null
    run?: string | null
    region?: string
    ctx?: { sketch?: unknown; instance?: unknown }
  } | null
  try {
    v = JSON.parse(json)
  } catch {
    return null
  }
  if (v === null || typeof v !== 'object') return null
  // A pick that hit a detached sketch names that sketch's own context (the
  // kernel's `ctx`); absent means the request's own context.
  const own = v.ctx !== undefined && v.ctx !== null && typeof v.ctx.sketch === 'number'
    ? { ctx: ctxOfSketchOnly(v.ctx.sketch, typeof v.ctx.instance === 'number' ? v.ctx.instance : null) }
    : {}
  if (typeof v.edge === 'string' && typeof v.unit === 'string') {
    return { edge: v.edge, unit: v.unit, curve: typeof v.curve === 'string' ? v.curve : null, run: typeof v.run === 'string' ? v.run : null, ...own }
  }
  if (typeof v.region === 'string') return { region: v.region, ...own }
  return null
}

// ── Refs ────────────────────────────────────────────────────────────────────

export type DraftKind = 'draft-island' | 'draft-curve' | 'draft-edge' | 'draft-region'

/** The selection ref of a unit, curve, edge, or region of `ctx`. */
export function draftRef(kind: DraftKind, ctx: string, key: string): NodeRef {
  return { kind, id: keyToId(key), ctx, key }
}

export function isDraftRef(n: NodeRef): n is NodeRef & { ctx: string; key: string } {
  return (
    (n.kind === 'draft-island' || n.kind === 'draft-curve' || n.kind === 'draft-edge' || n.kind === 'draft-region') &&
    n.ctx !== undefined &&
    n.key !== undefined
  )
}

const near = (p: V3, q: V3, eps: number): boolean =>
  Math.abs(p[0] - q[0]) <= eps && Math.abs(p[1] - q[1]) <= eps && Math.abs(p[2] - q[2]) <= eps

function pointSegDist(p: V3, a: V3, b: V3): number {
  const abx = b[0] - a[0], aby = b[1] - a[1], abz = b[2] - a[2]
  const l2 = abx * abx + aby * aby + abz * abz
  const t = l2 > 0 ? Math.max(0, Math.min(1, ((p[0] - a[0]) * abx + (p[1] - a[1]) * aby + (p[2] - a[2]) * abz) / l2)) : 0
  return Math.hypot(p[0] - (a[0] + abx * t), p[1] - (a[1] + aby * t), p[2] - (a[2] + abz * t))
}

/** The drawing edges that make up a region's rings (a solid's own boundary
 *  edges are not drawing edges and are skipped). Matched by geometry: each
 *  ring segment's midpoint lies on the drawing edge. */
export function regionRingEdges(pu: PlaneUnits, region: PlaneRegion, outerOnly = false): PlaneEdge[] {
  const rings = outerOnly ? [region.outer] : [region.outer, ...region.holes]
  const all: PlaneEdge[] = []
  for (const u of pu.units) all.push(...u.edges)
  const hit = new Set<string>()
  const out: PlaneEdge[] = []
  for (const ring of rings) {
    const n = ring.length
    for (let i = 0; i < n; i++) {
      const a = ring[i], b = ring[(i + 1) % n]
      const mid: V3 = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2]
      const len = Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2])
      const eps = Math.max(1e-7, len * 1e-6)
      for (const e of all) {
        if (hit.has(e.key)) continue
        // A ring segment can span several collinear drawing edges and an
        // edge can span several ring segments: match by midpoint-on-edge in
        // either direction.
        const emid: V3 = [(e.a[0] + e.b[0]) / 2, (e.a[1] + e.b[1]) / 2, (e.a[2] + e.b[2]) / 2]
        if (pointSegDist(mid, e.a, e.b) <= eps || pointSegDist(emid, a, b) <= eps) {
          hit.add(e.key)
          out.push(e)
        }
      }
    }
  }
  return out
}

/** Whether every segment of `ring` lies along drawing edges (sampled along
 *  its length, so a segment spanning several collinear edges still counts).
 *  A segment on a solid's own boundary edge has no drawing edge under it. */
export function ringAllDrawn(pu: PlaneUnits, ring: readonly V3[]): boolean {
  const all: PlaneEdge[] = []
  for (const u of pu.units) all.push(...u.edges)
  const n = ring.length
  if (n < 2) return false
  for (let i = 0; i < n; i++) {
    const a = ring[i], b = ring[(i + 1) % n]
    const len = Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2])
    const eps = Math.max(1e-7, len * 1e-6)
    for (const t of [0.1, 0.3, 0.5, 0.7, 0.9]) {
      const p: V3 = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
      if (!all.some((e) => pointSegDist(p, e.a, e.b) <= eps)) return false
    }
  }
  return true
}

/** The unit a clicked region belongs to as a SHAPE, or `null` when the click
 *  is on the solid's own face. A region over no face (free drawing) is a
 *  shape, as on the ground. A region over a face is a shape only when EVERY
 *  edge of its outer ring is a drawing edge (an inner sub-face, a pinched
 *  owned shape); a ring that includes any of the solid's own edges (the face
 *  around a flush shape, an L-shaped leftover, a half of a split face, a
 *  rectangle flush to a corner) is the solid's face. */
export function unitOfRegion(pu: PlaneUnits, region: PlaneRegion): PlaneUnit | null {
  if (region.over !== null && !ringAllDrawn(pu, region.outer)) return null
  const ring = regionRingEdges(pu, region, true)
  if (ring.length === 0) return null
  const keys = ring.map((e) => pu.edgeByKey.get(e.key)?.unit.key).filter((k): k is string => k !== undefined)
  if (keys.length === 0) return null
  keys.sort()
  return pu.unitByKey.get(keys[0]) ?? null
}

/** The selection ref a kernel pick resolves to — a curve for a circle's
 *  edge, an edge for a plain line, the region's SHAPE (its island) for a fill
 *  click; `null` for a click on a plain face region (background). */
export function refOfPick(scene: PlaneUnitsScene, requestCtx: string, pick: PlanePick): NodeRef | null {
  // A detached unit's refs carry ITS context, so every later edit of them
  // (move, erase, offset, vertex drag) runs in the sketch-only scope.
  const ctx = pick.ctx ?? requestCtx
  if ('edge' in pick) {
    // A click on a piece of a circle, arc, or polygon selects that RUN; a run
    // of a single facet is just a line (as a one-edge sketch chain always was).
    if (pick.run !== null) {
      const pu = readPlaneUnits(scene, ctx)
      const run = pu?.runByKey.get(pick.run)?.run
      if (run !== undefined && run.edges.length > 1) return draftRef('draft-curve', ctx, run.key)
    }
    return draftRef('draft-edge', ctx, pick.edge)
  }
  const pu = readPlaneUnits(scene, ctx)
  if (pu === null) return null
  const region = pu.regionByKey.get(pick.region)
  if (region === undefined) return null
  const unit = unitOfRegion(pu, region)
  return unit === null ? null : draftRef('draft-island', ctx, unit.key)
}

/** Pick the drawing under `ray` in `ctx`'s plane as a selection ref. */
export function pickDraftRef(
  scene: PlaneUnitsScene,
  ctx: string,
  ray: { origin: readonly number[]; direction: readonly number[] },
  tolerance: number,
): NodeRef | null {
  const pick = pickPlaneUnit(
    scene,
    ctx,
    new Float64Array([ray.origin[0], ray.origin[1], ray.origin[2], ray.direction[0], ray.direction[1], ray.direction[2]]),
    tolerance,
  )
  return pick === null ? null : refOfPick(scene, ctx, pick)
}

// ── Resolving a ref ─────────────────────────────────────────────────────────

/** What a draft ref stands for right now. `null` when stale. */
export interface ResolvedDraft {
  pu: PlaneUnits
  /** The unit the ref lies in (for a curve, the first); null for a region
   *  that has no drawing edge. */
  unit: PlaneUnit | null
  /** The edges the ref is made of (island: all; curve: the circle's facets;
   *  edge: itself; region: its drawing ring edges). */
  edges: PlaneEdge[]
}

export function resolveDraft(scene: PlaneUnitsScene, ref: NodeRef): ResolvedDraft | null {
  if (!isDraftRef(ref)) return null
  const pu = readPlaneUnits(scene, ref.ctx)
  if (pu === null) return null
  switch (ref.kind) {
    case 'draft-island': {
      const unit = pu.unitByKey.get(ref.key)
      return unit === undefined ? null : { pu, unit, edges: unit.edges }
    }
    case 'draft-edge': {
      const hit = pu.edgeByKey.get(ref.key)
      return hit === undefined ? null : { pu, unit: hit.unit, edges: [hit.edge] }
    }
    case 'draft-curve': {
      const edges = edgesOfCurveRef(pu, ref.key)
      if (edges.length === 0) return null
      return { pu, unit: pu.edgeByKey.get(edges[0].key)?.unit ?? null, edges }
    }
    case 'draft-region': {
      const region = pu.regionByKey.get(ref.key)
      if (region === undefined) return null
      const edges = regionRingEdges(pu, region)
      return { pu, unit: edges.length > 0 ? pu.edgeByKey.get(edges[0].key)?.unit ?? null : null, edges }
    }
    default:
      return null
  }
}

/** Whether `ref` still names something in the drawing. */
export function draftRefAlive(scene: PlaneUnitsScene, ref: NodeRef): boolean {
  if (!isDraftRef(ref)) return false
  const pu = readPlaneUnits(scene, ref.ctx)
  if (pu === null) return false
  switch (ref.kind) {
    case 'draft-island': return pu.unitByKey.has(ref.key)
    case 'draft-edge': return pu.edgeByKey.has(ref.key)
    case 'draft-region': return pu.regionByKey.has(ref.key)
    case 'draft-curve': return edgesOfCurveRef(pu, ref.key).length > 0
    default: return false
  }
}

/** The edge keys a ref stands for (a draft's `keys` argument). */
export function edgeKeysOf(scene: PlaneUnitsScene, ref: NodeRef): string[] {
  const r = resolveDraft(scene, ref)
  return r === null ? [] : r.edges.map((e) => e.key)
}

/** The unit (island) a ref lies in — what a transform moves for an edge or
 *  curve, exactly as a sketch edge/curve moved its island before. */
export function unitKeyOf(scene: PlaneUnitsScene, ref: NodeRef): string | null {
  return resolveDraft(scene, ref)?.unit?.key ?? null
}

/** The unit holding the edge with this key. */
export function unitOfEdge(pu: PlaneUnits, edgeKey: string): PlaneUnit | null {
  return pu.edgeByKey.get(edgeKey)?.unit ?? null
}

/** A ref's line work as flat world segment endpoints
 *  `[x0,y0,z0,x1,y1,z1,…]` — the highlight and the move ghost. */
export function geometryOf(scene: PlaneUnitsScene, ref: NodeRef): Float32Array {
  const r = resolveDraft(scene, ref)
  if (r === null) return new Float32Array(0)
  const out = new Float32Array(r.edges.length * 6)
  r.edges.forEach((e, i) => {
    out.set([e.a[0], e.a[1], e.a[2], e.b[0], e.b[1], e.b[2]], i * 6)
  })
  return out
}

/** Every store a ref's edges live in (deduplicated). */
export function storesOf(scene: PlaneUnitsScene, ref: NodeRef): DraftStore[] {
  const r = resolveDraft(scene, ref)
  if (r === null) return []
  const seen = new Set<string>()
  const out: DraftStore[] = []
  for (const e of r.edges) {
    const k = e.store.kind === 'free' ? `f${e.store.sketch}`
      : e.store.kind === 'owned' ? `o${e.store.object}:${e.store.sketch}`
      : `t${e.store.object}`
    if (seen.has(k)) continue
    seen.add(k)
    out.push(e.store)
  }
  return out
}

/** The objects a ref's topology or owned edges belong to (deduplicated). */
export function objectsOf(scene: PlaneUnitsScene, ref: NodeRef): bigint[] {
  const out: bigint[] = []
  for (const s of storesOf(scene, ref)) {
    if (s.kind !== 'free' && !out.includes(s.object)) out.push(s.object)
  }
  return out
}

/** Whether every edge of the ref is in a free sketch (a pure-void drawing). */
export function allFree(scene: PlaneUnitsScene, ref: NodeRef): boolean {
  const stores = storesOf(scene, ref)
  return stores.length > 0 && stores.every((s) => s.kind === 'free')
}

/** Segment endpoint pairs (world) of a ref — anchors for re-finding it by
 *  geometry after an edit. */
export function segmentsOf(scene: PlaneUnitsScene, ref: NodeRef): [V3, V3][] {
  const r = resolveDraft(scene, ref)
  return r === null ? [] : r.edges.map((e) => [e.a, e.b] as [V3, V3])
}

/** The units of `pu` that contain an edge matching one of `segments` (either
 *  direction, within `eps`). */
export function unitsAtSegments(pu: PlaneUnits, segments: readonly (readonly [V3, V3])[], eps = 1e-6): PlaneUnit[] {
  const out: PlaneUnit[] = []
  for (const u of pu.units) {
    const hit = u.edges.some((e) =>
      segments.some(([a, b]) => (near(e.a, a, eps) && near(e.b, b, eps)) || (near(e.a, b, eps) && near(e.b, a, eps))),
    )
    if (hit) out.push(u)
  }
  return out
}

/** Draft kinds as a runtime list. */
export const DRAFT_KINDS: readonly NodeKind[] = ['draft-island', 'draft-curve', 'draft-edge', 'draft-region']

// ── Naming ──────────────────────────────────────────────────────────────────

/** The Outliner / Object Info word for a drawn shape, derived from its edges
 *  and curve runs — nothing stores what tool drew it. The same words the
 *  imprint and sketch-island rows always used: a lone circle, arc, or regular
 *  polygon; a closed loop that is a rectangle, carries a drawn arc ("Arc
 *  shape"), or is any other "Shape"; a single segment "Line", a longer open
 *  path "Edge shape" (an "Arc" when it carries a drawn arc); else "Shape". */
export function unitName(unit: PlaneUnit): string {
  const n = unit.edges.length
  if (n === 0) return t({ context: 'drawn shape', message: 'Shape' })
  const q = (p: V3) => `${Math.round(p[0] * 1e6)},${Math.round(p[1] * 1e6)},${Math.round(p[2] * 1e6)}`
  const degree = new Map<string, number>()
  const adj = new Map<string, string[]>()
  for (const e of unit.edges) {
    const a = q(e.a), b = q(e.b)
    degree.set(a, (degree.get(a) ?? 0) + 1)
    degree.set(b, (degree.get(b) ?? 0) + 1)
    adj.set(a, [...(adj.get(a) ?? []), b])
    adj.set(b, [...(adj.get(b) ?? []), a])
  }
  // One connected component?
  const start = q(unit.edges[0].a)
  const seen = new Set<string>([start])
  const stack = [start]
  while (stack.length > 0) {
    for (const nb of adj.get(stack.pop() as string) ?? []) {
      if (!seen.has(nb)) {
        seen.add(nb)
        stack.push(nb)
      }
    }
  }
  const connected = seen.size === degree.size
  const degrees = [...degree.values()]
  const closed = connected && degrees.every((d) => d === 2)
  const path = connected && !closed && degrees.every((d) => d <= 2)
  const hasCircle = unit.curves.some((c) => c.kind === 'circle')
  const polygon = unit.curves.find((c) => c.kind === 'polygon')
  if (n === 1) return unit.curves.length > 0 && hasCircle ? t({ context: 'drawn shape', message: 'Arc' }) : t({ context: 'drawn shape', message: 'Line' })
  if (unit.curves.length === 1 && unit.curves[0].edges.length === n) {
    if (polygon !== undefined) return t({ context: 'drawn shape', message: 'Polygon' })
    return closed ? t({ context: 'drawn shape', message: 'Circle' }) : t({ context: 'drawn shape', message: 'Arc' })
  }
  if (closed) {
    if (hasCircle) return t({ context: 'drawn shape', message: 'Arc shape' })
    if (n === 4) {
      // Walk the loop in order to test the corners.
      const pts: V3[] = []
      let cur = unit.edges[0].a
      const used = new Set<string>()
      for (let i = 0; i < n; i++) {
        pts.push(cur)
        const next = unit.edges.find((e) => !used.has(e.key) && (near(e.a, cur, 1e-6) || near(e.b, cur, 1e-6)))
        if (next === undefined) break
        used.add(next.key)
        cur = near(next.a, cur, 1e-6) ? next.b : next.a
      }
      if (pts.length === 4 && isRectangle(pts)) return t({ context: 'drawn shape', message: 'Rectangle' })
    }
    return t({ context: 'drawn shape', message: 'Shape' })
  }
  if (path) return hasCircle ? t({ context: 'drawn shape', message: 'Arc' }) : t({ context: 'drawn shape', message: 'Edge shape' })
  return t({ context: 'drawn shape', message: 'Shape' })
}

/** A `draft-curve` run ref widened to its whole circle (the double-click
 *  gesture): the circle's curve key names every run. A `draft-edge` that is
 *  a one-edge run of a curve (a polygon's side cut between two crossings,
 *  which a single click picks as a line) widens the same way. Anything that
 *  is not a run of a curve crossed into several runs is returned unchanged. */
export function widenCurveRef(scene: PlaneUnitsScene, ref: NodeRef): NodeRef {
  if (!isDraftRef(ref) || (ref.kind !== 'draft-curve' && ref.kind !== 'draft-edge')) return ref
  const pu = readPlaneUnits(scene, ref.ctx)
  if (pu === null) return ref
  const run = ref.kind === 'draft-curve' ? pu.runByKey.get(ref.key)?.run : pu.runOfEdge.get(ref.key)
  if (run === undefined) return ref
  return runsOfCurve(pu, run.curveKey).length > 1 ? draftRef('draft-curve', ref.ctx, run.curveKey) : ref
}

// ── Outliner placement ──────────────────────────────────────────────────────

export interface OutlinerUnit {
  ref: NodeRef
  unit: PlaneUnit
  /** Distinct objects the unit's topology/owned edges belong to. */
  objects: bigint[]
}

export interface OutlinerDrawing {
  /** Units under their object (any topology or owned edge), keyed by the
   *  smallest owning object id (as a string). */
  byObject: Map<string, OutlinerUnit[]>
  /** Units entirely in free sketches, in sketch order. */
  free: OutlinerUnit[]
  /** Row label of every unit listed under an object, by `nodeKey` of its
   *  ref: its `unitName`, numbered 1-based when one object carries several
   *  of one name. */
  labels: Map<string, string>
}

/** The scene slice the outliner reads. `object_drawing_planes` finds the
 *  planes of topology-only drawing (a drawn shape with no sketch at all). */
export interface OutlinerScene extends PlaneUnitsScene {
  /** Every plane the free world drawing lies on with its listing, in one
   *  call (`Scene.plane_units_all_json`): `[{plane: [px,py,pz,nx,ny,nz],
   *  units}]`. Absent on an older host, which lists plane by plane. */
  plane_units_all_json?(): string
  sketch_ids(): ArrayLike<bigint>
  /** Every live world sketch, shown or not; the Outliner lists and numbers
   *  by this so hiding a group does not renumber the rest. */
  all_sketch_ids?(): ArrayLike<bigint>
  sketch_plane(sketch: bigint): Float64Array | undefined
  object_drawing_planes?(object: bigint): ArrayLike<number>
}

/** The planes `object` carries drawing on, in its OWN frame, as canonical
 *  `[px,py,pz,nx,ny,nz]` tuples. Empty for a stale or hidden object, or a
 *  scene that cannot answer. */
export function objectDrawingPlanes(
  scene: { object_drawing_planes?(object: bigint): ArrayLike<number> },
  object: bigint,
): [number, number, number, number, number, number][] {
  if (scene.object_drawing_planes === undefined) return []
  let flat: ArrayLike<number>
  try {
    flat = scene.object_drawing_planes(object)
  } catch {
    return []
  }
  const out: [number, number, number, number, number, number][] = []
  for (let i = 0; i + 5 < flat.length; i += 6) {
    const p = canonicalPlane([flat[i], flat[i + 1], flat[i + 2]], [flat[i + 3], flat[i + 4], flat[i + 5]])
    if (p !== null) out.push(p)
  }
  return out
}

/** The object-scoped context of one of `object`'s drawing planes: that plane
 *  in the object's own frame, only that object's drawing taking part. */
export function ctxOfObjectPlane(plane: readonly number[], object: bigint): string {
  return JSON.stringify({ plane: [...plane], object: Number(object) })
}

/** The shared contexts of every plane `object` carries drawing on, posed
 *  through `pose` (the instance's 3x4 world pose) for definition-member
 *  geometry. These are the same strings a click on that drawing resolves to,
 *  so refs built from them match picked ones. */
export function drawingCtxsOfObject(
  scene: { object_drawing_planes?(object: bigint): ArrayLike<number> },
  object: bigint,
  instance: bigint | null = null,
  pose: ArrayLike<number> | null = null,
): string[] {
  const out: string[] = []
  for (const p of objectDrawingPlanes(scene, object)) {
    let ctx = ctxOfPlane([p[0], p[1], p[2]], [p[3], p[4], p[5]], instance)
    if (ctx !== null && pose !== null) ctx = ctxThroughAffine(ctx, pose)
    if (ctx !== null && !out.includes(ctx)) out.push(ctx)
  }
  return out
}

/** One entry of `plane_units_all_json`: a plane's listing, or a detached
 *  sketch's own (`sketch` set — such a shape is in no plane's arrangement). */
interface AllListingEntry {
  plane: number[]
  sketch?: number
  detached?: boolean
  units: unknown
}

let allCache: { scene: unknown; gen: string | null; entries: AllListingEntry[] } | null = null

/** The whole document's free drawing in one kernel read, parsed once per
 *  document revision (the Outliner, Select All, a marquee, and the tools'
 *  detached-sketch checks all share it). */
function readAllListings(scene: { plane_units_all_json?(): string; history_generation?(): bigint }): AllListingEntry[] {
  if (scene.plane_units_all_json === undefined) return []
  const gen = genOf(scene as PlaneUnitsScene)
  if (allCache !== null && gen !== null && allCache.scene === scene && allCache.gen === gen) return allCache.entries
  let entries: AllListingEntry[] = []
  try {
    entries = JSON.parse(scene.plane_units_all_json()) as AllListingEntry[]
  } catch {
    entries = []
  }
  allCache = { scene, gen, entries }
  return entries
}

/** Every detached sketch of the world frame (separate-shapes design D16).
 *  Empty for a host without the batch listing. */
export function detachedSketchIds(scene: { plane_units_all_json?(): string; history_generation?(): bigint }): ReadonlySet<bigint> {
  const out = new Set<bigint>()
  for (const e of readAllListings(scene)) {
    if (e.detached === true && typeof e.sketch === 'number') out.add(BigInt(e.sketch))
  }
  return out
}

/** The drafting context a sketch's drawing is listed and edited in: its own
 *  sketch context when it is a detached shape (no plane arrangement holds
 *  it), else its plane's (`ctxOfSketch`). */
export function ctxOfSketchUnits(scene: WasmScene, sketch: bigint, instance: bigint | null = null): string | null {
  if (instance === null && detachedSketchIds(scene).has(sketch)) return ctxOfSketchOnly(sketch)
  return ctxOfSketch(scene, sketch, instance)
}

/** Where every drawing unit of the document lists in the Outliner: under its
 *  object when any edge is a topology or owned edge, else under its free
 *  sketch row. Planes are found from the sketches (free and owned) and from
 *  each given object's drawing planes. */
export function outlinerDrawing(scene: OutlinerScene, objects: readonly bigint[]): OutlinerDrawing {
  const ctxs: string[] = []
  const seen = new Set<string>()
  const add = (c: string | null): void => {
    if (c !== null && !seen.has(c)) {
      seen.add(c)
      ctxs.push(c)
    }
  }
  // The free drawing's planes come in one read when the host offers it,
  // each plane's listing seeded into the cache under its context, so the
  // loop below reads them back without a kernel call per plane: a model
  // of tens of thousands of small sketches (a furnished room's stray
  // edges) took minutes plane by plane.
  if (scene.plane_units_all_json !== undefined) {
    for (const entry of readAllListings(scene)) {
      // A detached shape lists as its own context, after the planes.
      const ctx = entry.detached === true && typeof entry.sketch === 'number'
        ? ctxOfSketchOnly(entry.sketch)
        : ctxOfPlane([entry.plane[0], entry.plane[1], entry.plane[2]], [entry.plane[3], entry.plane[4], entry.plane[5]])
      if (ctx === null) continue
      seedPlaneUnits(scene, ctx, parsePlaneUnits(ctx, JSON.stringify(entry.units)))
      add(ctx)
    }
  }
  // Without the batch, each sketch names its own plane. (With it, every
  // free sketch's plane is already listed, and a sketch's own context can
  // differ from its plane's at the last digit, which would read that plane
  // a second time, the slow way.)
  if (scene.plane_units_all_json === undefined) {
    const listed = scene.all_sketch_ids !== undefined ? scene.all_sketch_ids() : scene.sketch_ids()
    for (const sid of Array.from(listed)) add(ctxOfSketch(scene as unknown as WasmScene, sid))
  }
  for (const o of objects) {
    for (const c of drawingCtxsOfObject(scene, o)) add(c)
  }
  const byObject = new Map<string, OutlinerUnit[]>()
  const free: OutlinerUnit[] = []
  for (const ctx of ctxs) {
    const pu = readPlaneUnits(scene, ctx)
    if (pu === null) continue
    for (const unit of pu.units) {
      const objs: bigint[] = []
      for (const e of unit.edges) {
        if (e.store.kind !== 'free' && !objs.includes(e.store.object)) objs.push(e.store.object)
      }
      const item: OutlinerUnit = { ref: draftRef('draft-island', ctx, unit.key), unit, objects: objs }
      if (objs.length === 0) {
        free.push(item)
      } else {
        const owner = objs.reduce((m, o) => (o < m ? o : m)).toString()
        const list = byObject.get(owner) ?? []
        list.push(item)
        byObject.set(owner, list)
      }
    }
  }
  const labels = new Map<string, string>()
  for (const list of byObject.values()) {
    const named = numberedLabels(list.map((u) => unitName(u.unit)))
    list.forEach((u, i) => labels.set(`${u.ref.kind}:${u.ref.ctx}:${u.ref.key}`, named[i]))
  }
  return { byObject, free, labels }
}

/** A drawing row's name: one word per ref kind. */
export function draftKindLabel(kind: NodeKind): string {
  switch (kind) {
    case 'draft-island': return t({ context: 'drawn shape', message: 'Shape' })
    case 'draft-curve': return t({ context: 'drawn shape', message: 'Curve' })
    case 'draft-edge': return t({ context: 'drawn shape', message: 'Line' })
    case 'draft-region': return t({ context: 'drawn shape', message: 'Region' })
    default: return t({ context: 'drawn shape', message: 'Shape' })
  }
}

/** Number duplicate names 1-based ("Shape 1", "Shape 2"); unique ones stay bare. */
export function numberedLabels(names: readonly string[]): string[] {
  const counts = new Map<string, number>()
  for (const n of names) counts.set(n, (counts.get(n) ?? 0) + 1)
  const seen = new Map<string, number>()
  return names.map((n) => {
    if ((counts.get(n) ?? 0) <= 1) return n
    const k = (seen.get(n) ?? 0) + 1
    seen.set(n, k)
    return `${n} ${k}`
  })
}
