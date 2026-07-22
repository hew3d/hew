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
   * A drawn curve's own analytic point: a circle's/arc's exact center, one of
   * its covered quadrant points, an anchored tangent, or a regular polygon's
   * drawn center. It names the CHAIN, not an edge — a center lies on no edge
   * at all — so it resolves through the curve's edges rather than `edge`.
   */
  | { kind: 'sketch-curve'; sketch: bigint; curve: bigint }
  /**
   * No selectable provenance: the snap is a CUE, not a target — the world
   * origin, a guide point, an axis, or a bare sketch vertex (all outrank
   * OnEdge/OnFace but denote nothing to select). The caller resolves what is
   * actually under the ray.
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
  // A drawn curve's analytic point (center / quadrant / tangent / polygon
  // center). These outrank the facet edges crowding around them — and, since
  // Center and Quadrant pull with extra gravity, they win the snap from
  // noticeably further out than the geometry they describe — so without this
  // branch a click at or near a drawn circle's rim or center fell through to
  // the ray re-probe and selected whatever region was under the cursor.
  if (
    snap.elementKind === 'sketch-curve' &&
    snap.sketch !== undefined &&
    snap.sketchCurve !== undefined
  ) {
    return { kind: 'sketch-curve', sketch: snap.sketch, curve: snap.sketchCurve }
  }
  return { kind: 'fallback' }
}

/** The subset of the wasm `Scene` the resolver probes for a `fallback` snap. */
export interface SelectScene {
  sketch_curve_chain(sketch: bigint, edge: bigint): BigUint64Array | bigint[]
  sketch_curve_edges(sketch: bigint, curve: bigint): BigUint64Array | bigint[]
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

/** A drawn curve CHAIN → the ref a click on that curve selects, or null when
 * the chain has no live edges (a stale handle).
 *
 * Routed deliberately through `edgeRef` rather than minting a ref from the
 * curve handle directly: a `NodeRef{kind: 'sketch-curve'}` is identified by a
 * representative EDGE everywhere else in the app (the Outliner, Object Info,
 * transform lifting), so an analytic-point click and a rim-edge click must
 * produce the *same* ref, not two spellings of one curve. For an intact circle
 * every facet shares one chain, so any facet's `edgeRef` canonicalizes to the
 * same representative — but `sketch_curve_edges` returns slotmap order, not id
 * order, so we pick the chain's LOWEST-id edge explicitly to keep the ref
 * deterministic and to match the representative `edgeRef` (via
 * `curve_chain_at`, which returns ascending) settles on.
 *
 * When sticky rules have split one curve into several chains that still share
 * the curve id (a line drawn across a circle), the analytic center is
 * genuinely shared by both arcs; it resolves to whichever chain holds the
 * lowest-id edge. A rim click on a different fragment selects that fragment
 * instead — an accepted ambiguity, since the center belongs to neither arc
 * more than the other, and both refs expose the same curve's Segments control.
 * When only one edge survives, `edgeRef` degrades to that `sketch-edge`. */
function curveRef(scene: SelectScene, sketch: bigint, curve: bigint): NodeRef | null {
  const edges = scene.sketch_curve_edges(sketch, curve)
  if (edges.length === 0) return null
  let lowest = edges[0]
  for (const e of edges) if (e < lowest) lowest = e
  return edgeRef(scene, sketch, lowest)
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
    case 'sketch-curve': {
      // Same context rule as the other sketch kinds. A curve whose edges are
      // all gone yields null here and falls through to the ray re-probe,
      // rather than selecting nothing at all.
      if (topLevel) {
        const ref = curveRef(deps.scene, pick.sketch, pick.curve)
        if (ref !== null) return ref
      }
      break
    }
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
