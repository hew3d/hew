/**
 * drawPlane — the drawing plane a draw tool (Line/Rectangle/Circle/Arc) is
 * currently anchored to, per the sketches-on-any-plane design
 * (the sketch-planes design §4).
 *
 * A `DrawPlane` names the plane a gesture commits into: the ground plane
 * (today's only option), a hovered sketch's own plane (sketch mode), or —
 * from Phase 3 on — an idle-locked arbitrary plane. `ground: true` marks the
 * literal ground plane so tools can keep their exact legacy z=0 arithmetic
 * (bit-identical committed coordinates, state hashes, and recordings) instead
 * of routing every ground point through basis math.
 */

import type { V3 } from '../viewport/geoHelpers'
import { applyAffine3x4, facePlaneBasis, rayPlaneIntersect } from '../viewport/geoHelpers'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { SketchTarget } from './sketchGesture'
import type { EditContext, Snap } from './types'

/** A drawing plane: any point on it, its unit normal, and a right-handed
 *  in-plane basis (u, v) with `cross(u, v) === normal`. */
export interface DrawPlane {
  origin: V3
  normal: V3
  u: V3
  v: V3
  /** True only for the literal ground plane (origin [0,0,0], normal
   *  [0,0,1]) — callers use this to select the legacy z=0 fast path instead
   *  of basis math, so ground-mode arithmetic is unchanged by this module's
   *  existence. */
  ground: boolean
}

/** Tolerance for "this point/normal describes the ground plane". Mirrors the
 *  kernel's plane-membership tolerance (`kernel::tol::PLANE_DIST`); shared by
 *  every ground-vs-plane test in the app (this module, `sketchGesture.ts`). */
export const GROUND_PLANE_EPS = 1e-9

/**
 * True iff the plane through `point` with unit `normal` IS the ground plane
 * (z = 0) as a point set: the normal is parallel to Z and the plane passes
 * through z = 0. Orientation-free on purpose — a flipped-but-coincident
 * plane (normal facing −Z) is still the ground plane for every ground-tool
 * point (z = 0 lands on it just as well). Shared by `planeFromSketch` below
 * and by `sketchGesture.ts`'s cached-handle "still on this plane" check.
 */
export function isGroundPlane(point: V3, normal: V3): boolean {
  const [nx, ny, nz] = normal
  const offset = nx * point[0] + ny * point[1] + nz * point[2]
  return (
    Math.abs(nx) <= GROUND_PLANE_EPS &&
    Math.abs(ny) <= GROUND_PLANE_EPS &&
    Math.abs(offset) <= GROUND_PLANE_EPS
  )
}

/**
 * The ground plane, with EXACT literal values — no float round-trip through
 * `facePlaneBasis` — so ground-mode arithmetic built on this plane is
 * bit-identical to the pre-Phase-2 hardcoded-z=0 code.
 */
export function groundDrawPlane(): DrawPlane {
  return { origin: [0, 0, 0], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0], ground: true }
}

/**
 * The `DrawPlane` for a live sketch, from its kernel plane
 * (`Scene.sketch_plane`). Returns the exact `groundDrawPlane()` (not a
 * float-derived equivalent) when the sketch's plane IS the ground plane, so
 * a sketch that happens to lie on the ground still gets the legacy fast
 * path. Returns `null` for a stale/hidden sketch (`sketch_plane` undefined)
 * or a degenerate plane (shouldn't happen for a live sketch, but
 * `facePlaneBasis` is the authority).
 *
 * `instance`, when set, means `handle` is DEFINITION-owned: `sketch_plane`
 * then answers in DEFINITION-local space, not world (`sketch_plane` has no
 * `_in_instance` sibling — unlike `sketch_edge_endpoints`, which explicitly
 * refuses a def-owned sketch rather than silently answer in the wrong
 * frame). The plane is posed forward through `instance` before being
 * returned, so every caller gets a genuinely WORLD-space `DrawPlane` either
 * way — mirrors `sketchGesture.ts`'s `isStillOnPlane`, including its
 * approximate (linear-part, re-normalized) normal transform rather than the
 * exact inverse-transpose a non-uniform-scale pose would technically need;
 * see that function's doc for why that's safe here (this only gates
 * SKETCH-MODE adoption — every actual commit still goes through the
 * kernel's own exact pose⁻¹ mapping regardless of this plane's precision).
 * Returns `null` (never falls back to the raw local plane) for a
 * stale/unknown instance or a degenerate posed normal.
 */
export function planeFromSketch(
  wasmScene: WasmScene,
  handle: bigint,
  instance: bigint | null = null,
): DrawPlane | null {
  const plane = wasmScene.sketch_plane(handle)
  if (plane === undefined) return null
  let [px, py, pz, nx, ny, nz] = plane
  if (instance !== null) {
    const pose = wasmScene.instance_pose(instance)
    if (pose === undefined) return null
    ;[px, py, pz] = applyAffine3x4(pose, [px, py, pz])
    const rnx = pose[0] * nx + pose[1] * ny + pose[2] * nz
    const rny = pose[4] * nx + pose[5] * ny + pose[6] * nz
    const rnz = pose[8] * nx + pose[9] * ny + pose[10] * nz
    const len = Math.hypot(rnx, rny, rnz)
    if (len <= 1e-12) return null
    ;[nx, ny, nz] = [rnx / len, rny / len, rnz / len]
  }
  const origin: V3 = [px, py, pz]
  const normal: V3 = [nx, ny, nz]
  if (isGroundPlane(origin, normal)) return groundDrawPlane()
  const basis = facePlaneBasis(normal)
  if (basis === null) return null
  return { origin, normal, u: basis.u, v: basis.v, ground: false }
}

/**
 * The `DrawPlane` through `through` with unit normal along world axis
 * `axis` (0=X, 1=Y, 2=Z) — the idle plane lock (Phase 3; implemented now
 * because it's trivial and shares this module's shape). Uses the exact
 * ground frame (u=[1,0,0], v=[0,1,0]) when the result IS the ground plane
 * (axis === 2 and `through` already has z = 0), matching `groundDrawPlane`'s
 * basis instead of `facePlaneBasis`'s (which would also be valid, just a
 * different in-plane rotation).
 */
export function axisDrawPlane(axis: 0 | 1 | 2, through: V3): DrawPlane {
  const normal: V3 = axis === 0 ? [1, 0, 0] : axis === 1 ? [0, 1, 0] : [0, 0, 1]
  if (axis === 2 && through[2] === 0) {
    return { origin: through, normal, u: [1, 0, 0], v: [0, 1, 0], ground: true }
  }
  // An axis-aligned unit normal never degenerates facePlaneBasis; the
  // fallback below is unreachable but keeps this total without a non-null
  // assertion.
  const basis = facePlaneBasis(normal) ?? { u: [1, 0, 0] as V3, v: [0, 1, 0] as V3 }
  return { origin: through, normal, u: basis.u, v: basis.v, ground: false }
}

/** Ray∩plane, delegating to the shared implementation in geoHelpers.ts. */
export function pointOnPlane(ray: Ray, plane: DrawPlane): V3 | null {
  return rayPlaneIntersect(ray.origin, ray.direction, plane.origin, plane.normal)
}

/**
 * Canonical cache key for `plane`: sign-normalizes the normal (the first
 * component with |c| > 1e-12 is made positive, flipping the others to
 * match — a plane and its flipped-normal twin describe the same point set)
 * then rounds the normal's three components and the scalar offset
 * (normal·origin, taken AFTER sign normalization) to 9 decimals. Two
 * `DrawPlane`s that describe the same geometric plane — regardless of which
 * point was used as `origin` or which way the normal faces — always collide
 * on this key; `groundDrawPlane()` and any plane merely coplanar with it
 * produce the identical string.
 */
export function planeKey(plane: DrawPlane): string {
  const [nx0, ny0, nz0] = plane.normal
  let sign = 1
  if (Math.abs(nx0) > 1e-12) sign = nx0 < 0 ? -1 : 1
  else if (Math.abs(ny0) > 1e-12) sign = ny0 < 0 ? -1 : 1
  else if (Math.abs(nz0) > 1e-12) sign = nz0 < 0 ? -1 : 1

  const nx = nx0 * sign
  const ny = ny0 * sign
  const nz = nz0 * sign
  const offset =
    sign *
    (nx0 * plane.origin[0] + ny0 * plane.origin[1] + nz0 * plane.origin[2])

  const r = (x: number) => x.toFixed(9)
  return `${r(nx)},${r(ny)},${r(nz)},${r(offset)}`
}

/**
 * The drawing-plane cue a draw tool should show RIGHT NOW (design §6 bullet
 * 1): a subtle finite grid patch on the active plane, so the user sees where
 * a non-ground gesture is about to land. Two cases produce a cue; everything
 * else is `null` (the world grid already covers the ground plane):
 *
 * - Anchored on a NON-ground plane (face mode or plane/sketch mode): the
 *   frozen plane, through the gesture's anchor point.
 * - Idle with an active arrow-key plane lock AND a tracked hover point: the
 *   locked axis plane through that hover point (so the cue previews where
 *   the plane would land if the user clicked now).
 *
 * Each tool calls this from its own `activeDrawPlaneCue()`, passing its own
 * anchored-plane/anchor pair (or nulls when not anchored) and idle-lock
 * state — the four draw tools share this one implementation instead of
 * reimplementing the same two-case dispatch four times.
 */
export function drawPlaneCue(params: {
  /** The frozen plane of whichever stage (face or plane mode) is anchored, or null if neither is. */
  anchoredPlane: DrawPlane | null
  /** The anchor point paired with `anchoredPlane` (ignored when that's null). */
  anchoredThrough: V3 | null
  /** The idle arrow-key plane lock's axis, or null if unlocked. */
  idleLock: 0 | 1 | 2 | null
  /** The last-tracked hover point while idle-locked, or null before any hover. */
  idleHover: V3 | null
}): { plane: DrawPlane; through: V3 } | null {
  if (params.anchoredPlane !== null && params.anchoredThrough !== null) {
    return params.anchoredPlane.ground
      ? null
      : { plane: params.anchoredPlane, through: params.anchoredThrough }
  }
  if (params.idleLock !== null && params.idleHover !== null) {
    const plane = axisDrawPlane(params.idleLock, params.idleHover)
    // A Z lock through a hover point that happens to sit at z=0 resolves to
    // the exact ground plane (mirrors the click-time `begin_ground_sketch`
    // fast path) — still no cue; the world grid already covers it.
    return plane.ground ? null : { plane, through: params.idleHover }
  }
  return null
}

/**
 * Memoizes the sketch-mode resolution for the CURRENT pointer event —
 * mirrors `FacePickCache` in faceDraw.ts. Keyed by reference equality on the
 * `Ray` passed in (the Viewport builds one Ray object per event); a miss
 * just recomputes.
 *
 * Two memos, because both halves are paid more than once per event. The
 * Viewport calls `snapConstraint(ray)` before EVERY pointer event and then,
 * on a click, the tool's `onPointerDown` resolves the same ray again — so
 * without this, one click runs the whole resolution twice, and every idle
 * move runs it once. `pickFor` memoizes the `pick_sketch` raycast;
 * `targetFor` memoizes the full resolved `{plane, target}` including
 * `rayLandsOnSketch`'s `sketch_lines` marshal and per-segment scan, which
 * is the expensive half on a dense sketch (a 96-facet circle, a hand-traced
 * profile).
 */
export class SketchPickCache {
  private cache: { ray: Ray; handle: bigint | null } | null = null
  private targetCache: { ray: Ray; resolved: { plane: DrawPlane; target: SketchTarget } } | null =
    null

  pickFor(wasmScene: WasmScene, ray: Ray): bigint | null {
    if (this.cache !== null && this.cache.ray === ray) {
      return this.cache.handle
    }
    const pick = wasmScene.pick_sketch(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    const handle = pick === undefined ? null : pick
    this.cache = { ray, handle }
    return handle
  }

  /** Memoized `resolve` for `ray`, computed via `compute` on a miss. Only
   *  `resolveIdleDrawTarget` calls this; it is a method rather than a free
   *  memo so the whole per-event cache clears as one (`clear`). */
  targetFor(
    ray: Ray,
    compute: () => { plane: DrawPlane; target: SketchTarget },
  ): { plane: DrawPlane; target: SketchTarget } {
    if (this.targetCache !== null && this.targetCache.ray === ray) {
      return this.targetCache.resolved
    }
    const resolved = compute()
    this.targetCache = { ray, resolved }
    return resolved
  }

  clear(): void {
    this.cache = null
    this.targetCache = null
  }
}

/**
 * Half-angle (radians) of the pick cone `Scene::pick_sketch` uses —
 * `SKETCH_PICK_APERTURE` in `crates/wasm-api/src/lib.rs`, mirrored here so
 * `raysLandsOnSketch` can measure the cone's radius in world units. Keep the
 * two in step.
 */
export const SKETCH_PICK_APERTURE = 0.02

/** Squared distance from `p` to the segment `a`–`b`. */
function distSqToSegment(p: V3, a: V3, b: V3): number {
  const abx = b[0] - a[0]
  const aby = b[1] - a[1]
  const abz = b[2] - a[2]
  const lenSq = abx * abx + aby * aby + abz * abz
  let t = 0
  if (lenSq > 0) {
    t = ((p[0] - a[0]) * abx + (p[1] - a[1]) * aby + (p[2] - a[2]) * abz) / lenSq
    t = t < 0 ? 0 : t > 1 ? 1 : t
  }
  const dx = p[0] - (a[0] + abx * t)
  const dy = p[1] - (a[1] + aby * t)
  const dz = p[2] - (a[2] + abz * t)
  return dx * dx + dy * dy + dz * dz
}

/**
 * True iff the ray genuinely LANDS ON `sketch` — the gate that turns a
 * `pick_sketch` hit into sketch-mode adoption.
 *
 * `pick_sketch` models the pick as a cone around the ray axis, measuring the
 * PERPENDICULAR distance from the axis to a sketch edge. That measure is
 * blind to the angle the ray meets the sketch's plane at: on a plane seen at
 * incidence θ, a perpendicular miss of `d` is an IN-PLANE miss of `d/sin θ`.
 * A vertical sketch viewed from a typical 3/4 camera runs θ ≈ 30°, so an edge
 * a comfortable half-metre away in its own plane still falls inside the cone
 * — and the tool would silently adopt that plane for a click the user aimed
 * at the ground half a metre away, sending every later point in the gesture
 * off to wherever the ray happens to pierce the adopted plane.
 *
 * So: adopt only when the point the ray actually PIERCES the sketch's plane
 * at is within the pick cone's own radius (`SKETCH_PICK_APERTURE × distance
 * along the ray`) of the sketch's geometry. Measuring at the pierce point
 * puts the tolerance in the plane the user would be drawing on, which is the
 * plane the question is about; scaling it by distance keeps the gate
 * screen-sized, matching how the cone itself behaves. A ray parallel to the
 * plane (no pierce point) never lands on it.
 *
 * `instance`, when set, means `sketch` is DEFINITION-owned and `plane` has
 * already been posed forward into WORLD space (`planeFromSketch`'s
 * `instance` param) — `sketch_lines` itself still answers in DEFINITION-
 * local coordinates (no `_in_instance` sibling exists), so each segment
 * endpoint is mapped through the SAME pose before being measured against
 * `hit`, which is already a real WORLD point (`pointOnPlane` against the
 * now-WORLD `plane`). Without this, every segment would be compared in the
 * wrong frame — exactly the bug this function's `instance` param exists to
 * close (component-edit-parity.md phase A2).
 */
export function rayLandsOnSketch(
  wasmScene: WasmScene,
  sketch: bigint,
  plane: DrawPlane,
  ray: Ray,
  instance: bigint | null = null,
): boolean {
  const hit = pointOnPlane(ray, plane)
  if (hit === null) return false

  let lines: ArrayLike<number>
  try {
    lines = wasmScene.sketch_lines(sketch)
  } catch {
    return false // stale handle — nothing to land on
  }
  if (lines.length < 6) return false

  const pose = instance !== null ? wasmScene.instance_pose(instance) : undefined
  if (instance !== null && pose === undefined) return false // stale instance — nothing to land on

  const dx = hit[0] - ray.origin[0]
  const dy = hit[1] - ray.origin[1]
  const dz = hit[2] - ray.origin[2]
  const alongRay = Math.sqrt(dx * dx + dy * dy + dz * dz)
  const toleranceSq = (SKETCH_PICK_APERTURE * alongRay) ** 2

  for (let i = 0; i + 5 < lines.length; i += 6) {
    let a: V3 = [lines[i], lines[i + 1], lines[i + 2]]
    let b: V3 = [lines[i + 3], lines[i + 4], lines[i + 5]]
    if (pose !== undefined) {
      a = applyAffine3x4(pose, a)
      b = applyAffine3x4(pose, b)
    }
    if (distSqToSegment(hit, a, b) <= toleranceSq) return true
  }
  return false
}

/** The instance handle to mint/adopt def-owned sketches through, or `null`
 *  for ordinary world-space drawing — `null` for every `EditContext` except
 *  `'instance'` (component-edit-parity.md phase A2: an object/group context
 *  has no def-owned-sketch concept of its own; only a component instance's
 *  shared definition does). */
function instanceOf(editContext: EditContext): bigint | null {
  return editContext.kind === 'instance' ? editContext.id : null
}

/** Whether `sketch` is a live member sketch of `component` — the gate for
 *  adopting a hovered sketch (SKETCH MODE) while inside an instance context:
 *  a world sketch merely visible nearby must not become the target (that
 *  would draw INTO the world, escaping the definition being edited — the
 *  mirror image of the axis-lock symptom this design fixes). */
function isDefMemberSketch(wasmScene: WasmScene, component: bigint, sketch: bigint): boolean {
  for (const s of wasmScene.component_member_sketches(component)) {
    if (s === sketch) return true
  }
  return false
}

/**
 * Resolve the plane/target an IDLE gesture would anchor onto at `ray`
 * (design §1/§4) — the one implementation the five plane-mode draw tools
 * (Line/Rectangle/Circle/Polygon/Arc) share.
 *
 * A top-level `pick_sketch` hit whose plane is non-ground adopts that sketch
 * (SKETCH MODE) — but only if the ray really lands on it (`rayLandsOnSketch`,
 * which see); otherwise the ground plane (PLANE MODE, the default). Callers
 * reach this only once face mode has been ruled out (face mode takes
 * priority), so there is no `activeContext` re-check here for an object/group
 * context.
 *
 * Inside an INSTANCE context (`editContext.kind === 'instance'`,
 * component-edit-parity.md phase A2): a hovered sketch adopts only when it is
 * a member of the entered definition (`isDefMemberSketch`); otherwise — and
 * for the empty-space PLANE MODE default — the target carries the instance
 * handle, so `runSketchGesture` mints/uses a DEFINITION-owned sketch
 * (`begin_sketch_on_plane_in_instance`) instead of a world one. This is the
 * fix for the "arrow-key idle lock ends up drawing in the world" symptom:
 * without it, plane mode always lands in `begin_ground_sketch()` regardless
 * of context.
 *
 * `isDefMemberSketch` is checked BEFORE the geometry below, not just as an
 * adoption gate: it is also what decides which FRAME that geometry is in. A
 * member sketch's `sketch_plane`/`sketch_lines` answer in DEFINITION-LOCAL
 * space and must be posed forward through `instance` before being compared
 * against the real WORLD-space `ray` (`planeFromSketch`/`rayLandsOnSketch`'s
 * own `instance` param, mirroring `sketchGesture.ts`'s `isStillOnPlane`) —
 * a plain world sketch needs no such mapping. Getting this backwards (map
 * unconditionally, or not at all) either corrupts a world sketch's plane or
 * leaves a member sketch's plane in the wrong frame, silently missing every
 * hover on a posed instance's own sketch and falling through to a fresh
 * ground-plane sketch instead.
 */
export function resolveIdleDrawTarget(
  wasmScene: WasmScene,
  sketchPickCache: SketchPickCache,
  ray: Ray,
  editContext: EditContext,
): { plane: DrawPlane; target: SketchTarget } {
  const instance = instanceOf(editContext)
  return sketchPickCache.targetFor(ray, () => {
    const sketchHandle = sketchPickCache.pickFor(wasmScene, ray)
    if (sketchHandle !== null) {
      const isEligible =
        instance === null ||
        (editContext.kind === 'instance' &&
          isDefMemberSketch(wasmScene, editContext.component, sketchHandle))
      if (isEligible) {
        // `instance` is exactly the right mapping frame here: null at top
        // level (no mapping — `sketchHandle` is a world sketch), or the
        // entered instance when `sketchHandle` was just confirmed to be one
        // of ITS definition's own members.
        const plane = planeFromSketch(wasmScene, sketchHandle, instance)
        if (
          plane !== null &&
          !plane.ground &&
          rayLandsOnSketch(wasmScene, sketchHandle, plane, ray, instance)
        ) {
          return { plane, target: { kind: 'existing', handle: sketchHandle, instance } }
        }
      }
    }
    const plane = groundDrawPlane()
    return { plane, target: { kind: 'plane', plane, instance } }
  })
}

/**
 * Resolve the plane/target the FIRST click of a gesture anchors onto
 * (design §5.2) — shared by the same five draw tools.
 *
 * An ACTIVE idle plane lock beats face pick and sketch-hover adoption: the
 * locked plane passes through `snap`'s point (free/unconstrained, per each
 * tool's `snapConstraint` idle-lock branch), so clicking a solid's corner
 * starts a vertical sketch at that corner. Falls back to
 * `resolveIdleDrawTarget` (sketch/ground) when no lock is active. Returns
 * `null` only when a lock is active but there is no snap point yet (nothing
 * to click through). Carries the instance handle through to the locked-plane
 * target too, same as `resolveIdleDrawTarget`'s plane-mode default.
 */
export function resolveClickDrawTarget(
  wasmScene: WasmScene,
  sketchPickCache: SketchPickCache,
  idlePlaneLock: 0 | 1 | 2 | null,
  snap: Snap | null,
  ray: Ray,
  editContext: EditContext,
): { plane: DrawPlane; target: SketchTarget } | null {
  if (idlePlaneLock !== null) {
    if (snap === null) return null
    const clickedPoint: V3 = [snap.x, snap.y, snap.z]
    const plane = axisDrawPlane(idlePlaneLock, clickedPoint)
    return { plane, target: { kind: 'plane', plane, instance: instanceOf(editContext) } }
  }
  return resolveIdleDrawTarget(wasmScene, sketchPickCache, ray, editContext)
}
