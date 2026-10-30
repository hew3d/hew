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
 * The returned ref is what a CLICK selects (a drawn edge/curve, a region's
 * shape, or a solid's node); a drag moves that ref, and the transform layer
 * already lifts a drawn edge/curve to the shape (island) it belongs to
 * (`transformSelection`), so the drag arm needs no separate edge→island step.
 *
 * Everything DRAWN resolves through one path (unified-drafting design §8):
 * the snap or ray names a PLANE (a solid face's plane, a sketch's plane, or
 * the ground), and the kernel's `pick_plane_unit` answers which drawing unit
 * — whatever store it lives in — is under the ray there.
 */

import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  ctxOfFace,
  ctxOfSketch,
  draftRef,
  objectsOf,
  pickDraftRef,
  readPlaneUnits,
  type PlaneUnitsScene,
} from './planeUnits'

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
  /** World endpoints `[ax,ay,az,bx,by,bz]` of a sketch edge, or `undefined`
   *  for a stale/def-owned handle. Optional so a caller with no location
   *  data (or a test double) degrades cleanly to `curveRef`'s tie path
   *  rather than failing to resolve at all. */
  sketch_edge_endpoints?(sketch: bigint, edge: bigint): Float64Array | number[] | undefined
  sketch_region_island(sketch: bigint, region: bigint): bigint | undefined
  pick_sketch_region(
    ox: number, oy: number, oz: number, dx: number, dy: number, dz: number,
  ): { sketch(): bigint; region(): bigint; depth(): number; free(): void } | undefined
  pick_sketch_edge(
    ox: number, oy: number, oz: number, dx: number, dy: number, dz: number,
  ): { sketch(): bigint; edge(): bigint; depth(): number; free(): void } | undefined
  pick_sketch_region_in_instance?(
    instance: bigint,
    ox: number, oy: number, oz: number, dx: number, dy: number, dz: number,
  ): { sketch(): bigint; region(): bigint; depth(): number; free(): void } | undefined
  pick_sketch_edge_in_instance?(
    instance: bigint,
    ox: number, oy: number, oz: number, dx: number, dy: number, dz: number,
  ): { sketch(): bigint; edge(): bigint; depth(): number; free(): void } | undefined
  pick_face(
    ox: number, oy: number, oz: number, dx: number, dy: number, dz: number,
  ): { object(): bigint; face?(): bigint; instance(): bigint | undefined; depth(): number; free(): void } | undefined
  /** A solid edge's endpoints (kept for test doubles; no longer read). */
  edge_endpoints?(object: bigint, edge: bigint): Float64Array | number[] | undefined
  /** A placement's 3x4 pose, for planes of definition-owned sketches. */
  instance_pose?(instance: bigint): Float64Array | number[] | undefined
  /** A sketch's plane `[px,py,pz,nx,ny,nz]`, or `undefined` when stale. */
  sketch_plane?(sketch: bigint): Float64Array | number[] | undefined
  /** A solid face's plane and normal, for the context of a struck face. */
  face_plane?(object: bigint, face: bigint): ArrayLike<number>
  face_normal?(object: bigint, face: bigint): ArrayLike<number>
  /** The drawing on a plane (planeUnits.ts). Optional so test doubles
   *  without it simply never resolve a drawn thing. */
  plane_units_json?(ctx: string): string
  pick_plane_unit?(ctx: string, ray: Float64Array, tolerance: number): string
  history_generation?(): bigint
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
  /** World size of one screen pixel at a world point, through the live
   *  camera (`CameraRig.worldPerPixel`): turns the chord pick's pixel radius
   *  into a world distance at whatever zoom and model scale is on screen. */
  worldPerPixelAt: (point: readonly [number, number, number]) => number
  /** Whether the shapes drawn on this object's faces may be picked even
   *  though the object itself resolves to nothing selectable — the entered
   *  object of an object context (its own face click deselects; its
   *  imprints still select). Same predicate the draw tools use. */
  imprintEligible?: (objectId: bigint, instanceId: bigint | undefined) => boolean
  /** Whether faces hide what is behind them — false in X-ray and Wireframe,
   *  where nothing occludes (the render mode's face policy). Omitted means
   *  they do (Shaded). */
  facesOcclude?: boolean
}

/** Screen distance (pixels) within which a click counts as ON a drawn line —
 *  the same radius as a construction guide's pick (`GUIDE_PICK_PX`). A screen
 *  distance, not a world one, so a line on a millimetre part and a line on a
 *  house are picked alike. */
const DRAFT_PICK_PX = 8

const planeScene = (deps: ResolveDeps): PlaneUnitsScene => deps.scene as unknown as PlaneUnitsScene
const wasm = (deps: ResolveDeps): WasmScene => deps.scene as unknown as WasmScene

/** Whether a drawn thing is selectable in the active editing context: at the
 * top level everything is; inside a context only drawing on an object the
 * context makes eligible (its topology or owned stores) is — a free-standing
 * sketch is out of scope there, exactly as it always was. */
function draftInScope(ref: NodeRef, instance: bigint | undefined, deps: ResolveDeps): boolean {
  if (deps.context.length === 0) return true
  if (deps.scene.plane_units_json === undefined) return false
  const objects = objectsOf(planeScene(deps), ref)
  return objects.some((o) => deps.imprintEligible?.(o, instance) === true)
}

/** The drawing under `ray` on the plane `ctx`, as a selection ref. */
function pickInCtx(
  ctx: string | null,
  ray: Ray,
  at: readonly [number, number, number] | undefined,
  instance: bigint | undefined,
  deps: ResolveDeps,
  /** False for a drawing the active instance's own scoped probe found: it is
   *  in scope by construction, free or not. */
  scoped = true,
): NodeRef | null {
  if (ctx === null || deps.scene.pick_plane_unit === undefined) return null
  const point = at ?? [ray.origin[0], ray.origin[1], ray.origin[2]]
  const tol = DRAFT_PICK_PX * deps.worldPerPixelAt(point)
  const ref = pickDraftRef(planeScene(deps), ctx, ray, tol)
  return ref !== null && (!scoped || draftInScope(ref, instance, deps)) ? ref : null
}

/** A drawn curve's analytic snap point (center / quadrant / tangent /
 * polygon center) → the curve RUN it belongs to, in the sketch's plane. The
 * nearest run edge to the snap position wins (a rim point lies on it; a
 * center lies within epsilon of the run's center), the lowest edge key on a
 * tie. */
function curveSnapRef(
  ctx: string | null,
  point: readonly [number, number, number],
  deps: ResolveDeps,
): NodeRef | null {
  if (ctx === null) return null
  const pu = readPlaneUnits(planeScene(deps), ctx)
  if (pu === null) return null
  let best: { run: string; edge: string; d: number } | null = null
  for (const u of pu.units) {
    for (const run of u.curves) {
      const dc = Math.hypot(point[0] - run.center[0], point[1] - run.center[1], point[2] - run.center[2])
      for (const k of run.edges) {
        const e = pu.edgeByKey.get(k)?.edge
        if (e === undefined) continue
        const d = Math.min(Math.sqrt(pointSegmentDistSq(point, e.a, e.b)), dc < 1e-6 && run.radius !== null ? 0 : Infinity)
        if (!Number.isFinite(d)) continue
        if (best === null || d < best.d - 1e-9 || (Math.abs(d - best.d) <= 1e-9 && e.key < best.edge)) {
          best = { run: run.key, edge: e.key, d }
        }
      }
    }
  }
  if (best === null) return null
  // A run of one facet is just a line.
  const only = pu.runByKey.get(best.run)?.run
  return only !== undefined && only.edges.length > 1
    ? draftRef('draft-curve', ctx, best.run)
    : draftRef('draft-edge', ctx, best.edge)
}

/** The drawn edge a sketch snap names, matched by its endpoints. */
function sketchEdgeRef(
  ctx: string | null,
  ends: ArrayLike<number> | undefined,
  deps: ResolveDeps,
): NodeRef | null {
  if (ctx === null || ends === undefined || ends.length < 6) return null
  const pu = readPlaneUnits(planeScene(deps), ctx)
  if (pu === null) return null
  const near = (a: readonly number[], i: number) =>
    Math.abs(a[0] - ends[i]) <= 1e-6 && Math.abs(a[1] - ends[i + 1]) <= 1e-6 && Math.abs(a[2] - ends[i + 2]) <= 1e-6
  for (const u of pu.units) {
    for (const e of u.edges) {
      if ((near(e.a, 0) && near(e.b, 3)) || (near(e.a, 3) && near(e.b, 0))) {
        const run = pu.runOfEdge.get(e.key)
        return run !== undefined && run.edges.length > 1
          ? draftRef('draft-curve', ctx, run.key)
          : draftRef('draft-edge', ctx, e.key)
      }
    }
  }
  return null
}

/** Squared distance from `p` to the segment `a`–`b`, world space. */
function pointSegmentDistSq(
  p: readonly [number, number, number],
  a: readonly [number, number, number],
  b: readonly [number, number, number],
): number {
  const abx = b[0] - a[0]
  const aby = b[1] - a[1]
  const abz = b[2] - a[2]
  const abLenSq = abx * abx + aby * aby + abz * abz
  const apx = p[0] - a[0]
  const apy = p[1] - a[1]
  const apz = p[2] - a[2]
  const t = abLenSq > 0 ? Math.max(0, Math.min(1, (apx * abx + apy * aby + apz * abz) / abLenSq)) : 0
  const dx = p[0] - (a[0] + abx * t)
  const dy = p[1] - (a[1] + aby * t)
  const dz = p[2] - (a[2] + abz * t)
  return dx * dx + dy * dy + dz * dz
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

/** A scoped sketch is selectable only where it is actually rendered: before
 * the far plane and, when faces occlude (Shaded), not behind the nearest
 * solid face. The relative depth skin
 * mirrors the inference layer's occlusion rule, keeping coplanar linework
 * selectable at every model scale without exposing sketches behind thin walls. */
function scopedSketchVisible(depth: number, ray: Ray, deps: ResolveDeps): boolean {
  if (!withinFarPlane(depth, ray, deps)) return false
  // X-ray and Wireframe: nothing occludes, so no face can hide the sketch.
  if (deps.facesOcclude === false) return true
  const face = deps.scene.pick_face(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  if (face === undefined) return true
  try {
    const faceDepth = face.depth()
    return depth <= faceDepth + Math.max(Math.abs(faceDepth) * 1e-6, 1e-9)
  } finally {
    face.free()
  }
}

/** Refine a resolved plain-object pick to the DRAWING under the cursor, if
 * the click landed on one (docs/design/editable-face-sketches.md §3.1: click
 * inside a drawn shape selects the shape; the object is one click away
 * anywhere else). Only a pick that resolved to the object ITSELF (`kind ===
 * 'object'`, same id as the hit — never a wrapping group or a non-entered
 * instance) is refined, so grouped/instanced solids keep their explicit
 * double-click editing step.
 *
 * The struck face names the plane (its world plane, through the instance
 * when there is one); the kernel's plane-unit pick says what is drawn under
 * the ray there — a line or curve within `DRAFT_PICK_PX` screen pixels, else
 * the shape whose fill the ray lands in. A click on bare face (or on the face
 * around a shape) refines to nothing and the object stands. Near is measured
 * from the snap point when the snap is on this object, else from `hitPoint`
 * — where the ray meets the struck face — so a click on a guide or axis
 * lying over a drawn line (a cue snap with no object) still finds the line
 * under it. `hitPoint` is world space. */
function refineToDraft(
  resolved: NodeRef | null,
  hitObject: bigint,
  hitFace: bigint | undefined,
  hitInstance: bigint | undefined,
  hitPoint: [number, number, number] | undefined,
  snap: Snap | null,
  ray: Ray,
  deps: ResolveDeps,
): NodeRef | null {
  const isPlainObject = resolved !== null && resolved.kind === 'object' && resolved.id === hitObject
  const isEnteredObject = resolved === null && deps.imprintEligible?.(hitObject, hitInstance) === true
  if (!isPlainObject && !isEnteredObject) return resolved
  if (deps.scene.plane_units_json === undefined || hitFace === undefined) return resolved
  const ctx = ctxOfFace(wasm(deps), hitObject, hitFace, hitInstance ?? null)
  const at: [number, number, number] | undefined =
    snap !== null && snap.object === hitObject ? [snap.x, snap.y, snap.z] : hitPoint
  if (ctx === null || deps.scene.pick_plane_unit === undefined) return resolved
  const point = at ?? [ray.origin[0], ray.origin[1], ray.origin[2]]
  const ref = pickDraftRef(planeScene(deps), ctx, ray, DRAFT_PICK_PX * deps.worldPerPixelAt(point))
  // Inside an editing context only drawing on an object the context makes
  // eligible is selectable: a free sketch coplanar with the face is not.
  return ref !== null && draftInScope(ref, hitInstance, deps) ? ref : resolved
}

/** Where `ray` meets a face picked `depth` along it (a distance along the
 * ray, whatever the direction's length), or undefined for a degenerate ray. */
function pointAlongRay(ray: Ray, depth: number): [number, number, number] | undefined {
  const [dx, dy, dz] = ray.direction
  const len = Math.hypot(dx, dy, dz)
  if (!(len > 0) || !Number.isFinite(depth)) return undefined
  const k = depth / len
  return [ray.origin[0] + dx * k, ray.origin[1] + dy * k, ray.origin[2] + dz * k]
}

/** The VISIBLE solid under the ray (bounded to the far plane), context-scoped,
 * or null. Shared by the `fallback` path, so a click and a drag are bounded
 * alike: a solid beyond the render far plane is not drawn, and selecting an
 * undrawn solid is the precursor to Delete/Move on geometry the user cannot
 * see — as much a trap for a click as for a drag. `pick_face` returns the
 * NEAREST face, so this rejects only when no solid is drawn at that pixel;
 * a click on a visible solid (a nearer face) is never rejected. */
function solidUnderRay(ray: Ray, deps: ResolveDeps, snap: Snap | null = null): NodeRef | null {
  const facePick = deps.scene.pick_face(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  if (facePick === undefined) return null
  try {
    if (!withinFarPlane(facePick.depth(), ray, deps)) return null
    const object = facePick.object()
    const instance = facePick.instance()
    const resolved = deps.resolveObject(object, instance)
    const hit = pointAlongRay(ray, facePick.depth())
    return refineToDraft(resolved, object, facePick.face?.(), instance, hit, snap, ray, deps)
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
  const deepest = topLevel ? undefined : deps.context[deps.context.length - 1]
  const activeInstance = deepest?.kind === 'instance' ? deepest.id : undefined
  const sketchCtx = (sketch: bigint, instance: bigint | undefined): string | null => sketchCtxOf(sketch, instance, deps)
  // Definition sketches are rendered through the active instance but are not
  // duplicated into the global inference index per placement. Probe that
  // scoped geometry before interpreting a solid snap, otherwise the member
  // face beneath a visible sketch would always win and the sketch would
  // remain unselectable. Edge first preserves the usual "line on a filled
  // region selects the line; interior selects the region" behavior. The
  // probe finds the sketch (so its plane); the plane-unit pick resolves the
  // drawing.
  if (
    activeInstance !== undefined &&
    deps.scene.pick_sketch_region_in_instance !== undefined &&
    deps.scene.pick_sketch_edge_in_instance !== undefined
  ) {
    const edgePick = deps.scene.pick_sketch_edge_in_instance(
      activeInstance,
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (edgePick !== undefined) {
      try {
        if (scopedSketchVisible(edgePick.depth(), ray, deps)) {
          const at = pointAlongRay(ray, edgePick.depth())
          const ref = pickInCtx(sketchCtx(edgePick.sketch(), activeInstance), ray, at, activeInstance, deps, false)
          if (ref !== null) return ref
        }
      } finally {
        edgePick.free()
      }
    }
    const regionPick = deps.scene.pick_sketch_region_in_instance(
      activeInstance,
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (regionPick !== undefined) {
      try {
        if (scopedSketchVisible(regionPick.depth(), ray, deps)) {
          const at = pointAlongRay(ray, regionPick.depth())
          const ref = pickInCtx(sketchCtx(regionPick.sketch(), activeInstance), ray, at, activeInstance, deps, false)
          if (ref !== null) return ref
        }
      } finally {
        regionPick.free()
      }
    }
  }
  const pick = classifySnapPick(snap)
  switch (pick.kind) {
    case 'object': {
      // An object snap (a corner, an edge, a face point) resolves to the
      // node as before, then refines to the drawing under the cursor — the
      // ray pick supplies the struck face the snap itself does not carry.
      const resolved = deps.resolveObject(pick.object, pick.instance)
      const refinable =
        (resolved !== null && resolved.kind === 'object' && resolved.id === pick.object) ||
        (resolved === null && deps.imprintEligible?.(pick.object, pick.instance) === true)
      if (!refinable) return resolved
      const facePick = deps.scene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      let hitFace: bigint | undefined
      let hitPoint: [number, number, number] | undefined
      if (facePick !== undefined) {
        try {
          if (facePick.object() === pick.object) {
            hitFace = facePick.face?.()
            hitPoint = pointAlongRay(ray, facePick.depth())
          }
        } finally {
          facePick.free()
        }
      }
      return refineToDraft(resolved, pick.object, hitFace, pick.instance, hitPoint, snap, ray, deps)
    }
    case 'sketch-region':
    case 'sketch-edge':
    case 'sketch-curve': {
      // A top-level drawing is out of scope inside a context → resolve the
      // in-context thing under the ray (the fallback), never the sketch.
      if (!topLevel || snap === null) break
      const ctx = sketchCtx(pick.sketch, undefined)
      let ref: NodeRef | null = null
      if (pick.kind === 'sketch-curve') {
        // A curve's analytic point lies on no edge (a center) or on one of
        // several crowding facets — find its circle from where it IS.
        ref = curveSnapRef(ctx, [snap.x, snap.y, snap.z], deps)
      } else if (pick.kind === 'sketch-edge') {
        ref = sketchEdgeRef(ctx, deps.scene.sketch_edge_endpoints?.(pick.sketch, pick.edge), deps)
      }
      ref ??= pickInCtx(ctx, ray, [snap.x, snap.y, snap.z], undefined, deps)
      if (ref !== null) return ref
      break
    }
    case 'fallback':
      break
    default:
      pick satisfies never
  }

  // A guide or an axis outranks a line lying along it, so the snap holds on
  // the guide while the cursor is still well off the line: what the snap
  // latched onto is the line under the snap point, so look there first.
  if (snap !== null && (snap.kind === 'on-guide' || snap.kind === 'on-axis')) {
    const ref = underRay(rayThrough(ray, [snap.x, snap.y, snap.z]), snap, topLevel, deps)
    if (ref !== null && (ref.kind === 'draft-edge' || ref.kind === 'draft-curve')) return ref
  }
  return underRay(ray, snap, topLevel, deps)
}

/**
 * Fallback (a provenance-less snap, or a sketch snap out of context): resolve
 * what is actually under the ray. Sketch pickers are top-level only (they find
 * the sketch, hence the plane); the solid pick is far-plane bounded and
 * context-scoped; with no sketch under the ray the drawing, if any, is on the
 * ground plane. The plane pick measures its screen reach where the ray meets
 * the drawing: a perspective camera's scale at its own position is zero.
 */
function underRay(ray: Ray, snap: Snap | null, topLevel: boolean, deps: ResolveDeps): NodeRef | null {
  if (topLevel) {
    const regionPick = deps.scene.pick_sketch_region(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (regionPick !== undefined) {
      try {
        const at = pointAlongRay(ray, regionPick.depth())
        const ref = pickInCtx(sketchCtxOf(regionPick.sketch(), undefined, deps), ray, at, undefined, deps)
        if (ref !== null) return ref
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
        const at = pointAlongRay(ray, edgePick.depth())
        const ref = pickInCtx(sketchCtxOf(edgePick.sketch(), undefined, deps), ray, at, undefined, deps)
        if (ref !== null) return ref
      } finally {
        edgePick.free()
      }
    }
  }
  return solidUnderRay(ray, deps, snap)
}

/** A sketch's plane context, or null: a scene double without `sketch_plane`
 * cannot name a sketch's plane. */
function sketchCtxOf(sketch: bigint, instance: bigint | undefined, deps: ResolveDeps): string | null {
  return deps.scene.sketch_plane === undefined ? null : ctxOfSketch(wasm(deps), sketch, instance ?? null)
}

/** `ray` moved sideways to pass through `point`, same direction: exact for an
 * orthographic camera, and off a perspective one's by a fraction of a pixel's
 * worth of angle across the snap radius. */
function rayThrough(ray: Ray, point: readonly [number, number, number]): Ray {
  const [dx, dy, dz] = ray.direction
  const len2 = dx * dx + dy * dy + dz * dz
  const w = [point[0] - ray.origin[0], point[1] - ray.origin[1], point[2] - ray.origin[2]]
  const t = len2 > 0 ? (w[0] * dx + w[1] * dy + w[2] * dz) / len2 : 0
  return { origin: [point[0] - dx * t, point[1] - dy * t, point[2] - dz * t], direction: [dx, dy, dz] }
}
