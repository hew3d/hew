/**
 * snapSelection — the SINGLE, COMPLETE resolver that turns a resolved
 * inference snap into the selectable NodeRef under the cursor. Both the Select
 * click (`SelectTool` → `handleSelect`) and the drag-move arm
 * (`pickTransformableUnderCursor`, and the transform tools' auto-select
 * through it) reduce to one call of `resolveSelectableRef`, so click, drag,
 * and hover can never diverge — every axis they used to disagree on
 * (provenance, editing-context scoping, the depth bound) lives here, in one
 * place. Adding a new provenance kind is a compile error until this resolver
 * handles it (`never`-checked switch).
 *
 * The returned ref is what a CLICK selects (a sketch edge/curve, a region's
 * island, or a solid's node); a drag moves that ref, and the transform layer
 * already lifts a sketch edge/curve to the island it belongs to
 * (`transformSelection`), so the drag arm needs no separate edge→island step.
 */

import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

export type SnapPick =
  | { kind: 'object'; object: bigint; instance?: bigint }
  | { kind: 'sketch-region'; sketch: bigint; region: bigint }
  | { kind: 'sketch-edge'; sketch: bigint; edge: bigint }
  /**
   * No selectable provenance: the snap is a CUE, not a target — the world
   * origin, a guide point, an axis, a bare sketch vertex, or a drawn curve's
   * rim center/quadrant/tangent (all outrank OnEdge/OnFace but denote nothing
   * to select). The caller resolves what is actually under the ray.
   */
  | { kind: 'fallback' }

/** Classify a resolved snap (or `null`) into the entity it selects. */
export function classifySnapPick(snap: Snap | null): SnapPick {
  if (snap === null) return { kind: 'fallback' }
  // Solid geometry: a face/edge/vertex snap all carry the owning Object.
  if (snap.object !== undefined) {
    return { kind: 'object', object: snap.object, instance: snap.instance }
  }
  // A drawn region's fill (occlusion-aware: a nearer region already beat any
  // solid behind it, and any edge it borders, in resolve).
  if (
    snap.elementKind === 'sketch-region' &&
    snap.sketch !== undefined &&
    snap.sketchRegion !== undefined
  ) {
    return { kind: 'sketch-region', sketch: snap.sketch, region: snap.sketchRegion }
  }
  // A committed sketch edge — including a partition shared between two regions,
  // or a bare open polyline that never closed into a region.
  if (
    snap.elementKind === 'sketch-edge' &&
    snap.sketch !== undefined &&
    snap.element !== undefined
  ) {
    return { kind: 'sketch-edge', sketch: snap.sketch, edge: snap.element }
  }
  return { kind: 'fallback' }
}

/** The subset of the wasm `Scene` the resolver probes for a `fallback` snap. */
export interface SelectScene {
  sketch_curve_chain(sketch: bigint, edge: bigint): BigUint64Array | bigint[]
  sketch_region_island(sketch: bigint, region: bigint): bigint | undefined
  pick_sketch_region(
    ox: number, oy: number, oz: number, dx: number, dy: number, dz: number,
  ): { sketch(): bigint; region(): bigint; free(): void } | undefined
  pick_sketch_edge(
    ox: number, oy: number, oz: number, dx: number, dy: number, dz: number,
  ): { sketch(): bigint; edge(): bigint; free(): void } | undefined
  pick_face(
    ox: number, oy: number, oz: number, dx: number, dy: number, dz: number,
  ): { object(): bigint; instance(): bigint | undefined; depth(): number; free(): void } | undefined
}

/** Everything `resolveSelectableRef` needs from its host, so the resolution is
 * a pure function of the snap + these — testable without a real Viewport. */
export interface ResolveDeps {
  scene: SelectScene
  /** The active editing-context path (empty at the top level). */
  context: readonly NodeRef[]
  /** Object → its selectable node in the active context (outermost group /
   * the instance / itself), or null when hidden or out of the context's
   * scope. Wraps `resolvePickToSelectable` plus the hidden-set filter. */
  resolveObject: (objectId: bigint, instanceId: bigint | undefined) => NodeRef | null
  /** Camera forward UNIT vector (world space) — for the AXIAL depth bound. */
  cameraForward: readonly [number, number, number]
  /** The render far plane (meters, AXIAL). Read live from the camera. */
  cameraFar: number
}

/** A drawn sketch region → its island NodeRef (or the whole sketch). */
function regionRef(scene: SelectScene, sketch: bigint, region: bigint): NodeRef {
  const island = scene.sketch_region_island(sketch, region)
  return island !== undefined
    ? { kind: 'sketch-island', id: island, sketch }
    : { kind: 'sketch', id: sketch }
}

/** A sketch edge → the CURVE it belongs to (an arc/circle's facets act as one)
 * else that single line — the exact ref the click has always selected. */
function edgeRef(scene: SelectScene, sketch: bigint, edge: bigint): NodeRef {
  const chain = scene.sketch_curve_chain(sketch, edge)
  return chain.length > 1
    ? { kind: 'sketch-curve', id: chain[0], sketch }
    : { kind: 'sketch-edge', id: edge, sketch }
}

/** Reject a solid hit beyond the render far plane. `depth` is the RADIAL
 * ray-distance; the far plane clips AXIALLY, so project onto the camera
 * forward: axial = depth · (unit ray dir · camera forward). */
function withinFarPlane(depth: number, ray: Ray, deps: ResolveDeps): boolean {
  const [dx, dy, dz] = ray.direction
  const len = Math.hypot(dx, dy, dz)
  if (len === 0) return false
  const [fx, fy, fz] = deps.cameraForward
  const cos = (dx * fx + dy * fy + dz * fz) / len
  return depth * cos <= deps.cameraFar
}

/** The VISIBLE solid under the ray (bounded to the far plane), context-scoped,
 * or null. Shared by the `fallback` path, so a click and a drag are bounded
 * alike: a solid beyond the render far plane is not drawn, and selecting an
 * undrawn solid is the precursor to Delete/Move on geometry the user cannot
 * see — as much a trap for a click as for a drag. `pick_face` returns the
 * NEAREST face, so this rejects only when no solid is drawn at that pixel;
 * a click on a visible solid (a nearer face) is never rejected. */
function solidUnderRay(ray: Ray, deps: ResolveDeps): NodeRef | null {
  const facePick = deps.scene.pick_face(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  if (facePick === undefined) return null
  try {
    if (!withinFarPlane(facePick.depth(), ray, deps)) return null
    return deps.resolveObject(facePick.object(), facePick.instance())
  } finally {
    facePick.free()
  }
}

/**
 * The selectable NodeRef under `ray` for a resolved `snap`, applying — in one
 * place, so click and drag agree by construction:
 *   1. provenance classification (`classifySnapPick`);
 *   2. editing-context scoping — a free-standing (top-level) sketch is out of
 *      scope while inside a Group/instance context, so it is NOT selectable
 *      there; the in-context solid under the ray is resolved instead, or null;
 *   3. the AXIAL far-plane depth bound on the fallback solid pick.
 * Returns null when nothing selectable is under the cursor.
 */
export function resolveSelectableRef(
  snap: Snap | null,
  ray: Ray,
  deps: ResolveDeps,
): NodeRef | null {
  const topLevel = deps.context.length === 0
  const pick = classifySnapPick(snap)
  switch (pick.kind) {
    case 'object':
      return deps.resolveObject(pick.object, pick.instance)
    case 'sketch-region':
      // A top-level sketch is out of scope inside a context → resolve the
      // in-context thing under the ray (the fallback), never the sketch.
      if (topLevel) return regionRef(deps.scene, pick.sketch, pick.region)
      break
    case 'sketch-edge':
      if (topLevel) return edgeRef(deps.scene, pick.sketch, pick.edge)
      break
    case 'fallback':
      break
    default:
      pick satisfies never
  }

  // Fallback (a provenance-less snap, or a sketch snap out of context):
  // resolve what is actually under the ray. Sketch pickers are top-level only;
  // the solid pick is far-plane bounded and context-scoped.
  if (topLevel) {
    const regionPick = deps.scene.pick_sketch_region(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (regionPick !== undefined) {
      try {
        return regionRef(deps.scene, regionPick.sketch(), regionPick.region())
      } finally {
        regionPick.free()
      }
    }
    const edgePick = deps.scene.pick_sketch_edge(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (edgePick !== undefined) {
      try {
        return edgeRef(deps.scene, edgePick.sketch(), edgePick.edge())
      } finally {
        edgePick.free()
      }
    }
  }
  return solidUnderRay(ray, deps)
}
