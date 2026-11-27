/**
 * regionPick — resolve a ray to a REGION of a plane's drawing
 * (docs/design/unified-drafting.md §4.6, §8).
 *
 * The plane is the drafting context, so Offset and Push/Pull no longer ask
 * "is this a face or a sketch region": they pick the plane the ray lands on
 * (the face under it, a free sketch's plane, or the ground), read that
 * plane's `plane_units_json` listing, and take the listing's region entry.
 * The entry says what the region IS to the kernel — `over.face` (a face of a
 * solid), `extrude` (a free profile to extrude), or `over` with no face (a
 * pinch the solid cannot hold) — and carries its rings in world coordinates.
 */

import type { V3 } from '../viewport/geoHelpers'
import {
  pickPlaneUnit,
  planeOfCtxIn,
  readPlaneUnits,
  regionRingEdges,
  type PlaneRegion,
  type PlaneUnits,
  type PlaneUnitsScene,
} from './planeUnits'

/** A region of a context's listing, with the plane it lies in. */
export interface PickedRegion {
  ctx: string
  pu: PlaneUnits
  region: PlaneRegion
  /** A point on the plane (the region's first outer vertex) and its unit
   *  normal, world space — the drag constraint and measurement plane. */
  point: V3
  normal: V3
}

function describe(scene: PlaneUnitsScene, ctx: string, pu: PlaneUnits, region: PlaneRegion): PickedRegion | null {
  const pl = planeOfCtxIn(scene, ctx)
  if (pl === null || region.outer.length < 3) return null
  return { ctx, pu, region, point: region.outer[0], normal: pl.normal }
}

/** The region whose boundary carries the drawing edge `edgeKey`: an OUTER
 *  ring first (a shape owns its outline), any ring otherwise. */
export function regionOfEdge(pu: PlaneUnits, edgeKey: string): PlaneRegion | null {
  for (const outerOnly of [true, false]) {
    for (const r of pu.regions) {
      if (regionRingEdges(pu, r, outerOnly).some((e) => e.key === edgeKey)) return r
    }
  }
  return null
}

/**
 * The region under `ray` in the plane `ctx`. A click within `edgeTolerance`
 * (world) of a drawing edge resolves to the region that edge bounds — one
 * click on the outline OR the fill starts a gesture; pass 0 to take only
 * fills. `null` when nothing is there or the context is stale.
 */
export function regionUnderRay(
  scene: PlaneUnitsScene,
  ctx: string,
  ray: { origin: readonly number[]; direction: readonly number[] },
  edgeTolerance: number,
): PickedRegion | null {
  const pu = readPlaneUnits(scene, ctx)
  if (pu === null) return null
  const pick = pickPlaneUnit(
    scene,
    ctx,
    new Float64Array([ray.origin[0], ray.origin[1], ray.origin[2], ray.direction[0], ray.direction[1], ray.direction[2]]),
    Math.max(0, edgeTolerance),
  )
  if (pick === null) return null
  // A detached shape's pick names its own sketch context (as `refOfPick`
  // resolves it); the key lives in THAT listing, not the request's.
  const pickCtx = pick.ctx ?? ctx
  const pickPu = pickCtx === ctx ? pu : readPlaneUnits(scene, pickCtx)
  if (pickPu === null) return null
  if ('region' in pick) {
    const region = pickPu.regionByKey.get(pick.region)
    return region === undefined ? null : describe(scene, pickCtx, pickPu, region)
  }
  const region = regionOfEdge(pickPu, pick.edge)
  return region === null ? null : describe(scene, pickCtx, pickPu, region)
}

/** The listing entry of a free profile named by its `(sketch, region)`
 *  handles (a `pick_sketch_region` hit). */
export function regionOfFreeProfile(
  scene: PlaneUnitsScene,
  ctx: string,
  sketch: bigint,
  regionHandle: bigint,
): PickedRegion | null {
  const pu = readPlaneUnits(scene, ctx)
  if (pu === null) return null
  const region = pu.regions.find((r) => r.extrude !== null && r.extrude.sketch === sketch && r.extrude.region === regionHandle)
  return region === undefined ? null : describe(scene, ctx, pu, region)
}

/** The region of `ctx` whose rings carry the segment `a`–`b` (a picked
 *  sketch edge), outer rings first. */
export function regionOfSegment(
  scene: PlaneUnitsScene,
  ctx: string,
  a: V3,
  b: V3,
): PickedRegion | null {
  const pu = readPlaneUnits(scene, ctx)
  if (pu === null) return null
  const eps = 1e-6
  const onSeg = (pt: V3, p: V3, q: V3): boolean => {
    const dx = q[0] - p[0], dy = q[1] - p[1], dz = q[2] - p[2]
    const l2 = dx * dx + dy * dy + dz * dz
    const t = l2 > 0 ? Math.max(0, Math.min(1, ((pt[0] - p[0]) * dx + (pt[1] - p[1]) * dy + (pt[2] - p[2]) * dz) / l2)) : 0
    return Math.hypot(pt[0] - (p[0] + dx * t), pt[1] - (p[1] + dy * t), pt[2] - (p[2] + dz * t)) <= eps
  }
  for (const outerOnly of [true, false]) {
    for (const r of pu.regions) {
      const rings = outerOnly ? [r.outer] : [r.outer, ...r.holes]
      for (const ring of rings) {
        for (let i = 0; i < ring.length; i++) {
          const p = ring[i], q = ring[(i + 1) % ring.length]
          if (onSeg(a, p, q) && onSeg(b, p, q)) return describe(scene, ctx, pu, r)
        }
      }
    }
  }
  return null
}
