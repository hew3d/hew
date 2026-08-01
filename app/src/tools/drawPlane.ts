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
import type { DrawingAxes } from './drawingAxes'
import { getDrawingAxes, isWorldIdentity, WORLD_DRAWING_AXES } from './drawingAxes'
import { nextIdlePlaneLock } from './moveInput'
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
 * True iff `point` lies on `plane` within `GROUND_PLANE_EPS` — the same
 * tolerance the kernel's own `PLANE_DIST` uses for `SketchError::
 * PointOffPlane`, so this predicts the kernel's verdict exactly rather than
 * using an looser/independent UI tolerance that could disagree with it in
 * either direction. Used by LineTool's mid-gesture re-homing (tool-parity
 * playtest2 §2b) to detect an off-plane locked point BEFORE committing,
 * instead of committing speculatively and catching the kernel's refusal.
 */
export function isPointOnDrawPlane(point: V3, plane: DrawPlane): boolean {
  const [nx, ny, nz] = plane.normal
  const dist =
    nx * (point[0] - plane.origin[0]) +
    ny * (point[1] - plane.origin[1]) +
    nz * (point[2] - plane.origin[2])
  return Math.abs(dist) <= GROUND_PLANE_EPS
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
 * The `DrawPlane` through `through` with unit normal along drawing-axis
 * `axis` (0=red/X, 1=green/Y, 2=blue/Z) of `frame` — the idle plane lock
 * (Phase 3), extended by the movable-drawing-axes design (tool-parity §4)
 * to read the CURRENT frame instead of literal world X/Y/Z. `frame`
 * defaults to `WORLD_DRAWING_AXES` so every existing call site/test that
 * doesn't pass one keeps its exact legacy behavior.
 *
 * Uses the exact ground frame (u=[1,0,0], v=[0,1,0]) — matching
 * `groundDrawPlane`'s basis instead of `facePlaneBasis`'s (which would also
 * be valid, just a different in-plane rotation) — ONLY when `frame` is
 * world identity AND the result IS the literal ground plane (axis === 2,
 * `through` already has z = 0): the one case that must stay bit-identical
 * to the pre-movable-axes fast path (committed coordinates, state hashes,
 * recordings). A moved frame never takes this branch, even when its own
 * blue axis happens to be +Z through a point at z = 0 — SketchUp's Axes
 * tool doesn't re-grid the world, so a moved frame's "ground-like" plane is
 * still just a drawing plane, not THE ground plane.
 */
export function axisDrawPlane(
  axis: 0 | 1 | 2,
  through: V3,
  frame: DrawingAxes = WORLD_DRAWING_AXES,
): DrawPlane {
  if (isWorldIdentity(frame) && axis === 2 && through[2] === 0) {
    return { origin: through, normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0], ground: true }
  }
  const normal: V3 = axis === 0 ? frame.x : axis === 1 ? frame.y : frame.z
  // An axis-aligned (frame-orthonormal) unit normal never degenerates
  // facePlaneBasis; the fallback below is unreachable but keeps this total
  // without a non-null assertion.
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
  /**
   * The current drawing-axes frame (tool-parity design §4), read ONLY by the
   * idle-lock case below — mirrors `resolveClickDrawTarget`, which threads
   * this same frame into its own `axisDrawPlane` call so the idle-lock
   * PREVIEW resolves `idleLock`'s axis against the CURRENT frame instead of
   * literal world X/Y/Z, landing on the exact plane a click would resolve
   * onto. Defaults to `WORLD_DRAWING_AXES` so the anchored-plane branch
   * (which never reads it — it already has a fully resolved `DrawPlane`)
   * doesn't need to pass anything.
   */
  frame?: DrawingAxes
}): { plane: DrawPlane; through: V3 } | null {
  if (params.anchoredPlane !== null && params.anchoredThrough !== null) {
    return params.anchoredPlane.ground
      ? null
      : { plane: params.anchoredPlane, through: params.anchoredThrough }
  }
  if (params.idleLock !== null && params.idleHover !== null) {
    const plane = axisDrawPlane(params.idleLock, params.idleHover, params.frame ?? WORLD_DRAWING_AXES)
    // A Z lock through a hover point that happens to sit at z=0 resolves to
    // the exact ground plane (mirrors the click-time `begin_ground_sketch`
    // fast path) — still no cue; the world grid already covers it. Only
    // reachable at world identity: a moved frame's blue axis is never THE
    // ground plane, per `axisDrawPlane`'s doc comment.
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
export function instanceOf(editContext: EditContext): bigint | null {
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
    const plane = axisDrawPlane(idlePlaneLock, clickedPoint, getDrawingAxes(wasmScene))
    return { plane, target: { kind: 'plane', plane, instance: instanceOf(editContext) } }
  }
  return resolveIdleDrawTarget(wasmScene, sketchPickCache, ray, editContext)
}

/**
 * Re-lock (or release) a PLANE-anchored gesture's drawing plane THROUGH ITS
 * OWN ANCHOR, in response to an arrow key pressed AFTER the first click —
 * tool-parity playtest2 §2a, the "smaller half" of Kurt's finding 1 ("I can
 * set an axis lock before I start drawing a shape but as soon as I click the
 * starting point, that ability goes away"). Shared by Rectangle/Circle/
 * Polygon (a single `anchored` stage) and Arc's `chord` stage (the anchor is
 * endpoint A) — every one of them commits NOTHING to the kernel until the
 * gesture's LAST click, so re-locking here is just swapping which plane/
 * target the tool computes the REST of the gesture's points against; unlike
 * LineTool's chain (§2b), there is no partially-committed kernel state to
 * re-home.
 *
 * `current` is the mid-gesture lock axis already armed (or null — the SAME
 * field a tool's idle plane lock uses, since it's the same user choice,
 * merely exercised at a different moment); `key` is the arrow just pressed.
 * `nextIdlePlaneLock`'s ordinary toggle semantics apply: the same axis
 * pressed again (or ArrowDown) clears the lock. A cleared lock REVERTS to
 * `natural` — the plane/target the gesture would have anchored onto at its
 * own first click had no lock been active (the caller captures this via
 * `resolveIdleDrawTarget` at that same click, alongside the actually-used
 * `resolveClickDrawTarget` result) — rather than to the ground plane, so
 * releasing the lock returns to face/sketch/ground exactly as the first
 * click would have resolved it unlocked.
 *
 * The anchor point is passed through unchanged by the caller — it lies on
 * the new plane by construction (`axisDrawPlane`'s `through` IS the plane's
 * origin), so no repositioning or re-snapping is needed.
 */
export function nextGestureLockPlane(
  current: 0 | 1 | 2 | null,
  key: string,
  anchor: V3,
  natural: { plane: DrawPlane; target: SketchTarget },
  frame: DrawingAxes,
  editContext: EditContext,
): { lock: 0 | 1 | 2 | null; plane: DrawPlane; target: SketchTarget } {
  const lock = nextIdlePlaneLock(current, key)
  if (lock === null) return { lock, plane: natural.plane, target: natural.target }
  const plane = axisDrawPlane(lock, anchor, frame)
  return { lock, plane, target: { kind: 'plane', plane, instance: instanceOf(editContext) } }
}

/**
 * The `natural` plane/target a tool's first click should record for
 * `nextGestureLockPlane` above, WITHOUT probing sketch-hover — a plane
 * through `anchor` (the point that first click actually resolved, on the
 * ACTIVE lock's plane) with the LITERAL ground orientation (normal
 * [0,0,1]), namespaced to `editContext`'s instance exactly like any other
 * plane-mode target.
 *
 * Used only when the first click itself happened WHILE an idle plane lock
 * was already active (design §5.2): `resolveClickDrawTarget`'s locked
 * branch deliberately never calls `resolveIdleDrawTarget` for that click —
 * "an active lock beats face pick and sketch-hover adoption" means the
 * probe itself never runs, not merely that its result is discarded — so
 * there is no genuine hover result to remember for a LATER mid-gesture
 * release to revert to.
 *
 * MUST be anchored through `anchor`, not the world origin: the locked click
 * that produced `anchor` built its plane via `axisDrawPlane` through the
 * CLICKED point, so `anchor` can sit anywhere in space (e.g. a corner
 * picked while locked to a vertical plane) — that is entirely normal, not
 * an edge case. A release fallback of `groundDrawPlane()` verbatim would not
 * contain that anchor; the rest of the gesture would then resolve against a
 * plane that silently drops the anchor's own elevation, computing the WRONG
 * shape with no refusal (tool-parity playtest-2 review finding A).
 *
 * MUST use the LITERAL world Z as the normal — `axisDrawPlane(2, anchor,
 * WORLD_DRAWING_AXES)`, not the CURRENT drawing-axes `frame` — because that
 * is what this is standing in for: the unlocked resolution `anchor`'s own
 * click would have gotten instead, i.e. `resolveIdleDrawTarget`'s
 * plane-mode default, which is frame-agnostic (it always falls back to the
 * literal `groundDrawPlane()`, never reading the drawing-axes frame at all).
 * A movable drawing-axes frame's blue axis is only x×y with no verticality
 * constraint (`Scene::set_axes`) — AxesTool lets a user park it anywhere —
 * so passing the live `frame` here would anchor the release fallback to a
 * TILTED plane while the real unlocked click always lands on literal
 * ground: a gesture anchored off-lock, then released, would commit onto a
 * plane no unlocked click could ever produce (tool-parity DELTA review
 * finding 1). `axisDrawPlane`'s own fast path still collapses this to the
 * exact literal ground plane whenever the anchor really is on it, so the
 * ordinary (unmoved-frame or z=0-anchor) case is unchanged either way.
 * Repeating the sketch-hover probe at release time (the key event has no
 * ray to do it with) or eagerly probing at click time (which would violate
 * the "never probes while locked" invariant above) are not options. When
 * the first click was UNLOCKED, by contrast, `resolveClickDrawTarget`'s own
 * result already ties resolveIdleDrawTarget's answer — callers reuse that
 * directly instead of calling this, at no extra cost.
 *
 * (Open design question, not resolved here: arguably the movable axes frame
 * SHOULD redefine the default drawing plane everywhere — SketchUp's own
 * Axes tool works that way — in which case `resolveIdleDrawTarget`'s
 * ground-literal fallback would be the thing to change, and this function
 * would then want to follow `frame` again. That is a scope expansion beyond
 * a fix round; this function instead stays consistent with today's actual
 * unlocked-click behavior.)
 */
export function groundNaturalTarget(
  editContext: EditContext,
  anchor: V3,
): { plane: DrawPlane; target: SketchTarget } {
  const plane = axisDrawPlane(2, anchor, WORLD_DRAWING_AXES)
  return { plane, target: { kind: 'plane', plane, instance: instanceOf(editContext) } }
}
