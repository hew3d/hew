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
import { applyAffine3x4, facePlaneBasis, rayPlaneIntersect, transformNormalThroughPose } from '../viewport/geoHelpers'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { DrawingAxes } from './drawingAxes'
import { getDrawingAxes, isWorldIdentity, WORLD_DRAWING_AXES } from './drawingAxes'
import { nextIdlePlaneLock } from './moveInput'
import { pickPolicyGeneration } from './pickPolicy'
import { detachedSketchIds } from './planeUnits'
import type { SketchTarget } from './sketchGesture'
import type { EditContext, Snap } from './types'
import { distanceOffPlane, FACE_PLANE_EPS_M, type HoverFaceRef } from './faceDraw'
import type { PinnedPlane, PlanePin } from './planePin'

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
 * way. The point maps through the pose; the normal maps through the
 * pose's inverse-transpose (`transformNormalThroughPose`, as Push/Pull's and
 * `faceDraw.ts`'s instance faces do), so a non-uniformly scaled instance
 * still yields a plane that holds the posed sketch: the plain linear part
 * would tilt the normal off it. The plane is not just an adoption gate: an
 * in-instance fill hover hands it to the idle plane patch, the snap's
 * constraint plane and the depth test against a nearer face.
 * Returns `null` (never falls back to the raw local plane) for a
 * stale/unknown or singular instance pose, or a degenerate posed normal.
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
    const posed = transformNormalThroughPose(pose, [nx, ny, nz])
    if (posed === null) return null
    ;[nx, ny, nz] = posed
  }
  const origin: V3 = [px, py, pz]
  const normal: V3 = [nx, ny, nz]
  if (isGroundPlane(origin, normal)) return groundDrawPlane()
  const basis = facePlaneBasis(normal)
  if (basis === null) return null
  return { origin, normal, u: basis.u, v: basis.v, ground: false }
}

/**
 * The `DrawPlane` through `origin` with unit `normal` — the general
 * point+normal constructor `axisDrawPlane` below doesn't cover (its normal
 * is always a literal drawing axis). Mirrors `planeFromSketch`'s tail: the
 * exact `groundDrawPlane()` when the result IS the ground plane, else
 * `null` for a degenerate normal.
 */
export function drawPlaneThrough(origin: V3, normal: V3): DrawPlane | null {
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

/** The camera elevation (degrees above level, looking square-on at a wall)
 *  below which empty-space drawing leaves the blue plane for the wall's —
 *  the one tunable of the view-plane design (§3.3, D3). */
export const GROUND_PREFERENCE_DEG = 20

/**
 * How much more squarely a red or green plane must face the camera than the
 * blue plane does before empty-space drawing takes it (view-plane design
 * §3.3): cot(`GROUND_PREFERENCE_DEG`) ≈ 2.747. Square-on to a wall the switch
 * comes at 20° above level; turned away from the wall it comes lower (15.4°
 * at the home view's azimuth, 14.4° corner-on), because the wall faces the
 * camera less. The blue plane wins every tie, so Hew's home view (28.5°),
 * Iso, and every three-quarter view above 20° stay on the ground.
 */
export const GROUND_PREFERENCE = 1 / Math.tan((GROUND_PREFERENCE_DEG * Math.PI) / 180)

/** The camera as the view-plane inference reads it: `dir`, its forward
 *  direction (any length), and `eye`, its position, both in world space.
 *  The draw tools build it from their `updateCamera` feed (`cameraView`). */
export interface CameraView {
  dir: V3
  eye: V3
}

/**
 * Which axis plane of `frame` faces a camera looking along `view.dir` (the
 * camera's forward direction, not the cursor ray — one answer per view; see
 * the view-plane design §8): 0 red, 1 green, 2 blue, named for the normal's
 * axis like the arrow-key lock. The red or green plane, whichever faces more
 * (`|dir · n|`; red on an exact tie), wins only when it faces more than
 * `GROUND_PREFERENCE` times as squarely as the blue plane; otherwise blue.
 * Orientation-free (absolute values), so a camera below the ground looking up
 * mirrors one above it. A zero or non-finite `dir` answers blue.
 *
 * Behind the eye (§3.3): a red or green plane through the frame's origin
 * that the camera faces but looks AWAY from — the eye already past it, or
 * standing in it — is not a plane the cursor can point at, so the answer
 * falls back to blue. With `n` the chosen normal, `s = (eye − origin)·n`
 * and `f = dir·n`, the plane is behind when `s·f >= 0`. Blue is never
 * second-guessed this way: it stays blue even when behind.
 *
 * No hysteresis (§3.5): a pure function of the view, so the same view always
 * names the same plane.
 */
export function inferredAxis(frame: DrawingAxes, view: CameraView): 0 | 1 | 2 {
  const dir = view.dir
  const len = Math.hypot(dir[0], dir[1], dir[2])
  if (!(len > 1e-12) || !Number.isFinite(len)) return 2
  const dot = (a: V3, n: V3): number => a[0] * n[0] + a[1] * n[1] + a[2] * n[2]
  const facing = (n: V3): number => Math.abs(dot(dir, n)) / len
  const fx = facing(frame.x)
  const fy = facing(frame.y)
  const fz = facing(frame.z)
  const side: 0 | 1 = fy > fx ? 1 : 0
  if (!((side === 1 ? fy : fx) > GROUND_PREFERENCE * fz)) return 2
  const n = side === 1 ? frame.y : frame.x
  const o = frame.origin
  const s = dot([view.eye[0] - o[0], view.eye[1] - o[1], view.eye[2] - o[2]], n)
  return s * dot(dir, n) >= 0 ? 2 : side
}

/** The axis plane `axis` of `frame` through the frame's own origin — step
 *  7's plane once `inferredAxis` has chosen. The exact `groundDrawPlane()`
 *  for blue at world identity, so that path stays bit-identical. */
function framePlane(frame: DrawingAxes, axis: 0 | 1 | 2): DrawPlane {
  if (axis === 2 && isWorldIdentity(frame)) return groundDrawPlane()
  return axisDrawPlane(axis, [frame.origin[0], frame.origin[1], frame.origin[2]], frame)
}

/**
 * The plane an idle draw tool's click in empty space lands on (view-plane
 * design §3, picker step 7): the axis plane of `frame` that faces a camera
 * along `view.dir` from `view.eye` (`inferredAxis`), through the frame's
 * origin.
 *
 * At world identity with blue chosen this IS `groundDrawPlane()` — the exact
 * literal — so every ground-mode coordinate, state hash, and recording is
 * unchanged. A moved frame's blue plane is the frame's own XY plane through
 * its origin (D1), built the way the arrow-key lock builds it
 * (`axisDrawPlane`), so it is never the ground fast path even when it
 * happens to coincide with z = 0.
 */
export function inferredAxisPlane(frame: DrawingAxes, view: CameraView): DrawPlane {
  return framePlane(frame, inferredAxis(frame, view))
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
 * Memoizes the sketch-mode resolution for the CURRENT pointer event —
 * mirrors `FacePickCache` in faceDraw.ts. Keyed by reference equality on the
 * `Ray` passed in (the Viewport builds one Ray object per event); a miss
 * just recomputes.
 *
 * Three memos, because every part is paid more than once per event. The
 * Viewport calls `snapConstraint(ray)` before EVERY pointer event and then,
 * on a click, the tool's `onPointerDown` resolves the same ray again — so
 * without this, one click runs the whole resolution twice, and every idle
 * move runs it once. `pickFor` memoizes the `pick_sketch` raycast,
 * `regionFor` the `pick_sketch_region` fill raycast (both survive a
 * `targetFor` re-resolve under a reused ray, e.g. a camera turn);
 * `targetFor` memoizes the full resolved `{plane, target}` including
 * `rayLandsOnSketch`'s `sketch_lines` marshal and per-segment scan, which
 * is the expensive half on a dense sketch (a 96-facet circle, a hand-traced
 * profile).
 */
export class SketchPickCache {
  private cache: { ray: Ray; gen: number; handle: bigint | null } | null = null
  private regionCache: { ray: Ray; gen: number; instance: bigint | null; region: SketchFillPick | null } | null = null
  private targetCache: { ray: Ray; gen: number; key: string; resolved: IdleDrawTarget } | null = null

  pickFor(wasmScene: WasmScene, ray: Ray): bigint | null {
    if (this.cache !== null && this.cache.ray === ray && this.cache.gen === pickPolicyGeneration()) {
      return this.cache.handle
    }
    const pick = wasmScene.pick_sketch(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    const handle = pick === undefined ? null : pick
    this.cache = { ray, gen: pickPolicyGeneration(), handle }
    return handle
  }

  /** Memoized fill pick: the sketch whose filled region the ray lands in
   *  (`pick_sketch_region`, or `pick_sketch_region_in_instance` inside an
   *  instance context, whose handles are definition-local members by
   *  construction), or `null` on a miss — always a miss in Wireframe,
   *  where a fill is no input target. Mirrors `PushPullTool._pickRegionFor`:
   *  keyed by `ray`, `instance` and the pick-policy generation; the wasm
   *  pick object is read and freed at once. */
  regionFor(wasmScene: WasmScene, ray: Ray, instance: bigint | null): SketchFillPick | null {
    if (
      this.regionCache !== null &&
      this.regionCache.ray === ray &&
      this.regionCache.instance === instance &&
      this.regionCache.gen === pickPolicyGeneration()
    ) {
      return this.regionCache.region
    }
    const pick =
      instance !== null
        ? wasmScene.pick_sketch_region_in_instance(
            instance,
            ray.origin[0], ray.origin[1], ray.origin[2],
            ray.direction[0], ray.direction[1], ray.direction[2],
          )
        : wasmScene.pick_sketch_region(
            ray.origin[0], ray.origin[1], ray.origin[2],
            ray.direction[0], ray.direction[1], ray.direction[2],
          )
    let region: SketchFillPick | null = null
    if (pick !== undefined) {
      try {
        region = { sketch: pick.sketch(), detached: pick.detached() }
      } finally {
        pick.free()
      }
    }
    this.regionCache = { ray, gen: pickPolicyGeneration(), instance, region }
    return region
  }

  /** Memoized `resolve` for `ray`, computed via `compute` on a miss. Only
   *  `resolveIdleDrawTarget` calls this; it is a method rather than a free
   *  memo so the whole per-event cache clears as one (`clear`). `key` must
   *  name every input besides `ray` that can change the answer while the
   *  same ray is reused (a re-resolve on a key press, a camera change
   *  between events that share a cached ray): the boundary snap point and
   *  the inferred plane's axis and frame, as `resolveIdleDrawTarget`
   *  builds it. */
  targetFor(
    ray: Ray,
    compute: () => IdleDrawTarget,
    key = '',
  ): IdleDrawTarget {
    if (this.targetCache !== null && this.targetCache.ray === ray && this.targetCache.gen === pickPolicyGeneration() && this.targetCache.key === key) {
      return this.targetCache.resolved
    }
    const resolved = compute()
    this.targetCache = { ray, gen: pickPolicyGeneration(), key, resolved }
    return resolved
  }

  clear(): void {
    this.cache = null
    this.regionCache = null
    this.targetCache = null
  }
}

/** What `SketchPickCache.regionFor` keeps of a fill pick: the owning sketch
 *  and whether it is a detached shape. */
export interface SketchFillPick {
  sketch: bigint
  detached: boolean
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
 * True iff the ray genuinely LANDS ON `sketch`'s geometry — the gate that
 * turns a `pick_sketch` EDGE hit into sketch-mode adoption. It measures only
 * the distance to the sketch's segments, so a ray through the middle of a
 * filled region does not land here; `resolveIdleDrawTarget` adopts that case
 * through its fill pick (`pick_sketch_region`) instead, which needs no such
 * gate because the kernel already tested the pierce point against the
 * region.
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

/** Whether `sketch` is a detached shape. Asks the kernel per handle (it also
 *  knows component-definition members, which the world-frame batch listing
 *  omits); a host without the accessor falls back to that listing. */
function isDetachedSketch(wasmScene: WasmScene, sketch: bigint): boolean {
  const direct = (wasmScene as { sketch_is_detached?: (s: bigint) => boolean }).sketch_is_detached
  if (typeof direct === 'function') return direct.call(wasmScene, sketch)
  return detachedSketchIds(wasmScene).has(sketch)
}

/** What `resolveIdleDrawTarget` (and an unlocked `resolveClickDrawTarget`)
 *  answers: the plane and the sketch target a click would anchor onto. */
export interface IdleDrawTarget {
  plane: DrawPlane
  target: SketchTarget
  /**
   * Set only when the plane's orientation came from the view (picker step 7,
   * the axis plane the view faces, over nothing; or a named point lifted
   * through that plane, `planeThroughNamedPoint`) AND is not the exact
   * ground plane: the axis
   * `inferredAxis` chose — 0 red, 1 green, 2 blue (blue only under a moved
   * drawing-axes frame, D1). This is what the tools' status-bar inference
   * line (shown for 0 and 1) keys on; a face, sketch, or ground answer
   * leaves it absent, so the ground result is unchanged in shape.
   */
  inferredAxis?: 0 | 1 | 2
  /** Set only when an ineligible face decided the plane (picker step 6),
   *  so the plane state can tell a face from a sketch or the inferred
   *  plane (`DrawPlaneTracker`). */
  onFace?: true
  /** The face that decided the plane (step 6), for the cursor cue's
   *  outline (view-die design §4). */
  face?: HoverFaceRef
}

/**
 * Resolve the plane/target an IDLE gesture would anchor onto at `ray`
 * (design §1/§4) — the one implementation the five plane-mode draw tools
 * (Line/Rectangle/Circle/Polygon/Arc) share.
 *
 * Over a sketch (view-plane design §2 step 4) the hover adopts that
 * sketch's plane (SKETCH MODE), in two tries: (4a) a `pick_sketch` edge hit
 * the ray really lands on (`rayLandsOnSketch`, which see), then (4b) the
 * nearest filled region the ray passes through (`pick_sketch_region`, its
 * `_in_instance` sibling inside an instance context). A detached shape is
 * never adopted either way. A sketch on the ground plane gives the literal
 * ground PLANE-MODE target instead, exactly the pre-inference ground
 * fallback, so a hovered ground sketch keeps the ground from any camera. At
 * the top level an ineligible face (`foreignFace`) strictly nearer than the
 * point the ray pierces the sketch's plane wins (step 6); one the sketch lies
 * on does not. Callers reach this only once face mode has been ruled out
 * (face mode takes priority), so there is no `activeContext` re-check here
 * for an object/group context.
 *
 * Inside an INSTANCE context (`editContext.kind === 'instance'`,
 * component-edit-parity.md phase A2): a hovered sketch adopts only when it is
 * a member of the entered definition (`isDefMemberSketch` for an edge pick;
 * the fill pick's `_in_instance` sibling returns only members); otherwise — and
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
 *
 * Over nothing (view-plane design §2 step 7) the plane is the axis plane
 * the view faces (`inferredAxisPlane`) when `view` — the camera's
 * forward direction and eye, from the tool's `updateCamera` feed — is given; the
 * result then carries `inferredAxis` whenever that plane is not the exact
 * ground. Without `view` the fallback is the literal ground plane, as
 * before the view-plane design, and the drawing-axes frame is not read.
 * The memo key includes the inferred axis and the frame, so a camera that
 * turned under a reused ray never serves the old plane.
 *
 * Last, `snap` (when given) may lift the plane through a named point in the
 * air (`planeThroughNamedPoint`, wireframe-faces-and-solids design §2.1).
 */
export function resolveIdleDrawTarget(
  wasmScene: WasmScene,
  sketchPickCache: SketchPickCache,
  ray: Ray,
  editContext: EditContext,
  foreignFace?: () => { point: V3; normal: V3; depth: number; face?: HoverFaceRef } | null,
  foreignKey = '',
  view: CameraView | null = null,
  snap: Snap | null = null,
): IdleDrawTarget {
  const resolved = resolveIdleDrawTargetAt(wasmScene, sketchPickCache, ray, editContext, foreignFace, foreignKey, view)
  // A snap to a named point is that point (§2.1): applied after the memo,
  // since it depends on the snap and the memo is keyed by the ray.
  if (!isNamedPointSnap(snap)) return resolved
  return planeThroughNamedPoint(resolved, snap, ray, wasmScene, view, editContext)
}

function resolveIdleDrawTargetAt(
  wasmScene: WasmScene,
  sketchPickCache: SketchPickCache,
  ray: Ray,
  editContext: EditContext,
  foreignFace: (() => { point: V3; normal: V3; depth: number; face?: HoverFaceRef } | null) | undefined,
  foreignKey: string,
  view: CameraView | null,
): IdleDrawTarget {
  const instance = instanceOf(editContext)
  const frame = view === null ? null : getDrawingAxes(wasmScene)
  const axis = frame === null || view === null ? null : inferredAxis(frame, view)
  const key = frame === null
    ? foreignKey
    : `${foreignKey}|${axis}|${frame.origin.join(',')}|${frame.x.join(',')}|${frame.y.join(',')}`
  return sketchPickCache.targetFor(ray, () => {
    // Adopt `handle`'s plane (picker step 4), or `null` to fall through.
    // `viaEdge`: the handle came from the edge pick (`pick_sketch`), which
    // needs the detached and membership gates and the landing check; a
    // fill pick has already answered those (its ray is inside a region,
    // its detached flag is read off the pick, and the `_in_instance`
    // variant only returns the entered definition's own members).
    const adopt = (handle: bigint, viaEdge: boolean): IdleDrawTarget | null => {
      if (viaEdge) {
        // A detached shape touches nothing: hovering it never adopts it as
        // the sketch to draw into (separate-shapes design D3), so a plain
        // shape drawn over it lands on the face or plane beneath, as if it
        // were not there. Only a detached drawing tool call continues one,
        // deliberately.
        if (isDetachedSketch(wasmScene, handle)) return null
        if (
          instance !== null &&
          !(editContext.kind === 'instance' && isDefMemberSketch(wasmScene, editContext.component, handle))
        ) {
          return null
        }
      }
      // `instance` is exactly the right mapping frame here: null at top
      // level (no mapping — `handle` is a world sketch), or the entered
      // instance when `handle` is one of ITS definition's own members.
      const plane = planeFromSketch(wasmScene, handle, instance)
      if (plane === null) return null
      if (viaEdge && !rayLandsOnSketch(wasmScene, handle, plane, ray, instance)) return null
      const hit = pointOnPlane(ray, plane)
      if (hit === null) return null
      // A sketch behind a face the ray struck first is not what the cursor
      // is on; one lying on that face (equal depth) still is.
      const sketchDepth = Math.hypot(hit[0] - ray.origin[0], hit[1] - ray.origin[1], hit[2] - ray.origin[2])
      const ff = editContext.kind === 'top' && foreignFace !== undefined ? foreignFace() : null
      if (ff !== null && sketchDepth > ff.depth + 1e-6) return null
      // A ground sketch gives the ground: the literal z = 0 plane-mode
      // target, bit-identical to the pre-inference ground fallback (and
      // still literal z = 0 under a moved drawing-axes frame, D1), so a
      // low camera's inferred vertical plane never replaces it.
      if (plane.ground) {
        const ground = groundDrawPlane()
        return { plane: ground, target: { kind: 'plane', plane: ground, instance } }
      }
      return { plane, target: { kind: 'existing', handle, instance } }
    }

    // 4a: a sketch edge within the pick aperture.
    const edgeHandle = sketchPickCache.pickFor(wasmScene, ray)
    if (edgeHandle !== null) {
      const adopted = adopt(edgeHandle, true)
      if (adopted !== null) return adopted
    }
    // 4b: inside a sketch's filled region. A detached fill is never adopted;
    // the pick returns only the nearest fill, so one hides whatever fill
    // lies behind it (the hover falls through to steps 6-7).
    const fill = sketchPickCache.regionFor(wasmScene, ray, instance)
    if (fill !== null && !fill.detached) {
      const adopted = adopt(fill.sketch, false)
      if (adopted !== null) return adopted
    }
    // A face of a group member or component instance, struck from outside at
    // the top level: draw on that face's world plane as free drawing (the
    // way SketchUp does), not on the ground beneath it.
    if (editContext.kind === 'top' && foreignFace !== undefined) {
      const ff = foreignFace()
      if (ff !== null) {
        const facePlane = drawPlaneThrough(ff.point, ff.normal)
        if (facePlane !== null) {
          const target: SketchTarget = { kind: 'plane', plane: facePlane, instance: null }
          return ff.face === undefined ? { plane: facePlane, target, onFace: true } : { plane: facePlane, target, onFace: true, face: ff.face }
        }
      }
    }
    if (frame === null || axis === null) {
      const plane = groundDrawPlane()
      return { plane, target: { kind: 'plane', plane, instance } }
    }
    // Over nothing: the axis plane the view faces (step 7), carrying the
    // instance handle exactly as the ground fallback does.
    const plane = framePlane(frame, axis)
    const target: SketchTarget = { kind: 'plane', plane, instance }
    return plane.ground ? { plane, target } : { plane, target, inferredAxis: axis }
  }, key)
}

/** The snap kinds that name one exact point (the engine's `point_kind`:
 *  Endpoint, Center, Quadrant, Intersection, Midpoint; a guide point is an
 *  Endpoint). Every other kind (on-edge, on-face, on-axis, plane, ground)
 *  lies somewhere along a line or surface and is not a point the cursor
 *  asked for. */
export const NAMED_POINT_KINDS: ReadonlySet<string> = new Set(['endpoint', 'midpoint', 'center', 'quadrant', 'intersection'])

/** Whether `snap` is a named point (wireframe-faces-and-solids design §2). */
export function isNamedPointSnap(snap: Snap | null): snap is Snap {
  return snap !== null && NAMED_POINT_KINDS.has(snap.kind)
}

/**
 * The §2.1 test alone: `snap` is a named point that is off the plane
 * (`planePoint`, `normal`) by more than `FACE_PLANE_EPS_M` and, when
 * `onlyIfNearer` (the default), nearer the eye than the place the cursor ray
 * meets the plane (or the ray does not meet it). A type guard: the snap is
 * non-null past it.
 *
 * The nearer-the-eye half exists because an Object's face can hide a point
 * behind it; a plane cannot (R18 a), so callers testing the ground, a sketch
 * plane, the inferred axis plane or a pin pass `onlyIfNearer: false`.
 */
export function namedPointLeavesPlane(snap: Snap | null, ray: Ray, planePoint: V3, normal: V3, onlyIfNearer = true): snap is Snap {
  if (!isNamedPointSnap(snap)) return false
  const point: V3 = [snap.x, snap.y, snap.z]
  if (Math.abs(distanceOffPlane(point, planePoint, normal)) <= FACE_PLANE_EPS_M) return false
  if (!onlyIfNearer) return true
  const hit = rayPlaneIntersect(ray.origin, ray.direction, planePoint, normal)
  if (hit === null) return true
  const dist = (p: V3): number => Math.hypot(p[0] - ray.origin[0], p[1] - ray.origin[1], p[2] - ray.origin[2])
  return dist(point) < dist(hit)
}

/**
 * The post-picker step (wireframe-faces-and-solids design §2.1 as amended by
 * R18): a snap to a named point is that point.
 *
 * - When the picker chose an Object face (`resolved.onFace`), the point lifts
 *   the plane only if it is off the face and nearer the eye than the ray's hit
 *   on it (a face can hide a point). Otherwise it keeps the old answer: the
 *   tool drops it onto the face and says "projected".
 * - When the picker chose the ground, a sketch's plane or the inferred axis
 *   plane, a named point off that plane always lifts it (R18 a): a plane
 *   hides nothing.
 * - At a named point of a free sketch, with the view's inferred axis red or
 *   green, the plane is the inferred axis plane THROUGH the point rather than
 *   the sketch's own (R18 b), unless the sketch's plane already has that
 *   orientation. A ground sketch keeps the ground from an ordinary (blue)
 *   view, and over its fill.
 *
 * The lifted plane is the camera-facing axis plane through the point
 * (`inferredAxis`; without `view`, the frame's blue axis). Through a point at
 * z = 0 in the world frame this is the exact ground plane (`axisDrawPlane`'s
 * fast path). Anything else (no snap, a snap that is not a named point, a
 * point already on the plane) returns `resolved` itself, so every existing
 * result is untouched.
 */
export function planeThroughNamedPoint(
  resolved: IdleDrawTarget,
  snap: Snap | null,
  ray: Ray,
  wasmScene: WasmScene,
  view: CameraView | null,
  editContext: EditContext,
): IdleDrawTarget {
  if (!isNamedPointSnap(snap)) return resolved
  const point: V3 = [snap.x, snap.y, snap.z]
  const { plane } = resolved
  const off = namedPointLeavesPlane(snap, ray, plane.origin, plane.normal, resolved.onFace === true)
  // R18 b: a sketch's plane (a ground sketch gives the literal ground) at a
  // named point, seen from a red or green view. `inferredAxis` set means the
  // plane IS step 7's; `onFace` a face's.
  const sketchAdopted = view !== null && resolved.inferredAxis === undefined && resolved.onFace !== true &&
    (resolved.target.kind === 'existing' || plane.ground)
  if (!off && !sketchAdopted) return resolved
  const frame = getDrawingAxes(wasmScene)
  const axis = view === null ? 2 : inferredAxis(frame, view)
  if (!off) {
    // On the sketch's plane already: keep it unless the view calls for a
    // red or green plane of another orientation.
    if (axis === 2) return resolved
    const n = axis === 0 ? frame.x : frame.y
    const parallel = Math.abs(plane.normal[0] * n[0] + plane.normal[1] * n[1] + plane.normal[2] * n[2]) > 1 - 1e-9
    if (parallel) return resolved
  }
  const lifted = axisDrawPlane(axis, point, frame)
  const target: SketchTarget = { kind: 'plane', plane: lifted, instance: instanceOf(editContext) }
  // Like step 7's result: the view chose this plane's orientation, so the
  // "Facing the ... plane" hint shows at the lifted corner (R21).
  return view !== null && !lifted.ground ? { plane: lifted, target, inferredAxis: axis } : { plane: lifted, target }
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
 * `view` is passed through to `resolveIdleDrawTarget` (the view-plane
 * inference over nothing); a lock never reads it.
 */
export function resolveClickDrawTarget(
  wasmScene: WasmScene,
  sketchPickCache: SketchPickCache,
  idlePlaneLock: 0 | 1 | 2 | null,
  snap: Snap | null,
  ray: Ray,
  editContext: EditContext,
  foreignFace?: () => { point: V3; normal: V3; depth: number; face?: HoverFaceRef } | null,
  foreignKey = '',
  view: CameraView | null = null,
): IdleDrawTarget | null {
  if (idlePlaneLock !== null) {
    if (snap === null) return null
    const clickedPoint: V3 = [snap.x, snap.y, snap.z]
    const plane = axisDrawPlane(idlePlaneLock, clickedPoint, getDrawingAxes(wasmScene))
    return { plane, target: { kind: 'plane', plane, instance: instanceOf(editContext) } }
  }
  return resolveIdleDrawTarget(wasmScene, sketchPickCache, ray, editContext, foreignFace, foreignKey, view, snap)
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
 * `nextGestureLockPlane` above, WITHOUT probing sketch-hover — the plane an
 * unlocked click in empty space would have taken, moved to pass through
 * `anchor` (the point that first click actually resolved, on the ACTIVE
 * lock's plane), namespaced to `editContext`'s instance exactly like any
 * other plane-mode target.
 *
 * Used only when the first click itself happened WHILE an idle plane lock
 * was already active (design §5.2): `resolveClickDrawTarget`'s locked
 * branch deliberately never calls `resolveIdleDrawTarget` for that click —
 * "an active lock beats face pick and sketch-hover adoption" means the
 * probe itself never runs, not merely that its result is discarded — so
 * there is no genuine hover result to remember for a LATER mid-gesture
 * release to revert to.
 *
 * MUST be anchored through `anchor`, not the frame origin: the locked click
 * that produced `anchor` built its plane via `axisDrawPlane` through the
 * CLICKED point, so `anchor` can sit anywhere in space (e.g. a corner
 * picked while locked to a vertical plane) — that is entirely normal, not
 * an edge case. A release fallback through the origin would not contain
 * that anchor; the rest of the gesture would then resolve against a plane
 * that silently drops the anchor's offset, computing the WRONG shape with
 * no refusal (tool-parity playtest-2 review finding A).
 *
 * The plane's ORIENTATION is the empty-space inference's (view-plane design
 * §6): pass `view` — the camera (forward and eye) AT THE FIRST CLICK — with the
 * current drawing-axes `frame` (both, or neither), and the plane takes
 * `inferredAxis`'s choice of the frame's axes, so releasing a lock from a
 * Front view lands on the green plane through the anchor, exactly where an
 * unlocked click would have gone, and under a moved frame the blue choice
 * follows the frame's own Z (D1), as the unlocked click now does.
 * `axisDrawPlane`'s fast path still collapses a world-identity blue plane
 * through a z = 0 anchor to the exact literal ground plane, so the ordinary
 * case stays bit-identical.
 *
 * Without `view` (omitted, or null before any camera feed) the plane
 * keeps its pre-inference orientation, the LITERAL world Z
 * (`axisDrawPlane(2, anchor, WORLD_DRAWING_AXES)`), matching a
 * `resolveIdleDrawTarget` called without a view direction. Repeating the
 * sketch-hover probe at release time (the key event has no ray to do it
 * with) or eagerly probing at click time (which would violate the "never
 * probes while locked" invariant above) are not options. When the first
 * click was UNLOCKED, by contrast, `resolveClickDrawTarget`'s own result
 * already IS the unlocked resolution — callers reuse that directly instead
 * of calling this, at no extra cost.
 */
export function groundNaturalTarget(
  editContext: EditContext,
  anchor: V3,
): { plane: DrawPlane; target: SketchTarget }
export function groundNaturalTarget(
  editContext: EditContext,
  anchor: V3,
  view: CameraView | null,
  frame: DrawingAxes,
): { plane: DrawPlane; target: SketchTarget }
export function groundNaturalTarget(
  editContext: EditContext,
  anchor: V3,
  view: CameraView | null = null,
  frame: DrawingAxes = WORLD_DRAWING_AXES,
): { plane: DrawPlane; target: SketchTarget } {
  const plane = view === null
    ? axisDrawPlane(2, anchor, WORLD_DRAWING_AXES)
    : axisDrawPlane(inferredAxis(frame, view), anchor, frame)
  return { plane, target: { kind: 'plane', plane, instance: instanceOf(editContext) } }
}

/**
 * What the view die, its pips and the cursor cue read from a plane-drawing
 * tool (view-die design §3.4): one derived value the five draw tools build
 * with `DrawPlaneTracker` below, so the widget and the cue never read the
 * cue layer and agree with the plane a click lands on.
 */
export interface DrawPlaneState {
  /** The plane the next click (idle) or the current gesture (anchored) uses. */
  plane: DrawPlane
  /** Pip that represents it. */
  pip: PlanePip
  hold: 'inferred' | 'locked' | 'pinned' | 'anchored'
  /** What the cursor is over; drives the cursor cue (§4). Null off-canvas.
   *  Under a hold (lock, pin, gesture) the tool runs no face or sketch pick,
   *  so the kind there is always `'space'` and `point` is the cursor ray's
   *  hit on the held plane (the patch sits at the cursor). */
  hover: null | {
    kind: DrawPlaneHoverKind
    /** Ray hit on the plane. */
    point: V3
    /** World-space outer loop of the leaned-to face, when planar. */
    faceLoop?: V3[]
  }
}

/** The four pips under the view die: the drawing axes' red, green and blue
 *  planes, and any other plane. */
export type PlanePip = 'red' | 'green' | 'blue' | 'custom'

/** What the cursor is over, for the cursor cue (view-die design §4). */
export type DrawPlaneHoverKind = 'space' | 'ground' | 'face' | 'sketch' | 'lean' | 'leanExtended'

/** How far (degrees) a plane's normal may sit off a drawing axis and still
 *  light that axis's pip (view-die design §3.1), either direction. */
export const PIP_AXIS_TOLERANCE_DEG = 0.5
const PIP_AXIS_TOL_COS = Math.cos((PIP_AXIS_TOLERANCE_DEG * Math.PI) / 180)

/**
 * The pip a plane with `normal` lights: red, green or blue when the normal
 * is parallel to `frame`'s X, Y or Z within `PIP_AXIS_TOLERANCE_DEG`,
 * either direction, else custom. The offset never matters: a Shift pin on a
 * cube's front face lights green. Follows a moved drawing-axes frame, as the
 * arrow locks do. A zero or non-finite normal is custom.
 */
export function planePip(normal: V3, frame: DrawingAxes = WORLD_DRAWING_AXES): PlanePip {
  const len = Math.hypot(normal[0], normal[1], normal[2])
  if (!(len > 1e-12) || !Number.isFinite(len)) return 'custom'
  const along = (a: V3): number => Math.abs(normal[0] * a[0] + normal[1] * a[1] + normal[2] * a[2]) / len
  if (along(frame.x) >= PIP_AXIS_TOL_COS) return 'red'
  if (along(frame.y) >= PIP_AXIS_TOL_COS) return 'green'
  if (along(frame.z) >= PIP_AXIS_TOL_COS) return 'blue'
  return 'custom'
}

/** The arrow key a red, green or blue pip stands for (→ red, ← green, ↑
 *  blue), or null for the custom pip. */
export function pipArrowKey(pip: PlanePip): 'ArrowRight' | 'ArrowLeft' | 'ArrowUp' | null {
  /* oxlint-disable lingui/no-unlocalized-strings -- key values are identifiers */
  switch (pip) {
    case 'red': return 'ArrowRight'
    case 'green': return 'ArrowLeft'
    case 'blue': return 'ArrowUp'
    case 'custom': return null
  }
  /* oxlint-enable lingui/no-unlocalized-strings */
}

/** How a draw tool's idle picker decided the plane at a hover: the shape
 *  each tool's `_idleHoverResolution` answers. `face` is the deciding face
 *  (steps 3, 5, 6), `target` the sketch target when the picker adopted a
 *  sketch (step 4), `inferredAxis` the empty-space inference (step 7). */
export interface IdleHoverResolution {
  plane: DrawPlane
  inferredAxis?: 0 | 1 | 2
  onFace?: true
  face?: HoverFaceRef
  target?: SketchTarget
}

/** What the cursor was over at an idle hover, from how the picker decided
 *  the plane (`IdleHoverResolution`). */
export function idleHoverKind(resolution: IdleHoverResolution): DrawPlaneHoverKind {
  const face = resolution.face
  if (face !== undefined) {
    if (!face.boundary) return 'face'
    return face.offObject ? 'leanExtended' : 'lean'
  }
  if (resolution.onFace === true) return 'face'
  if (resolution.target?.kind === 'existing') return 'sketch'
  // The ground as a point set, whatever built it: a moved frame's blue plane
  // only translated within the ground or turned about blue IS the ground
  // the world grid already shows.
  if (resolution.plane.ground || isGroundPlane(resolution.plane.origin, resolution.plane.normal)) return 'ground'
  return 'space'
}

/** How far apart (in the face's own frame) two tessellated vertex normals
 *  of one face may be and the face still count as flat. A planar face's
 *  tessellation writes the exact plane normal at every corner; a chord facet
 *  of a curved wall carries the wall's varying normals. */
const FLAT_FACE_NORMAL_EPS = 1e-4

/**
 * The world-space outer loops of hovered faces, for the cursor cue's
 * outline (view-die design §4), with a memo per document revision.
 *
 * `face_boundary` answers in the object's own frame, which is world space
 * for a world object or a group member (groups bake their transforms) and
 * definition space for a component member, so a face struck through an
 * instance is posed forward through that instance (`applyAffine3x4`), the
 * mapping `worldFacePlane` uses for its point. A curved face (a chord facet
 * of a cylinder wall) gets no loop: its outline would be a sliver of the
 * wall, so the cue shows the small patch instead. Flatness is read off the
 * object's tessellated normals (`object_mesh` + `face_mesh_range`), cached
 * per object and document revision (`history_generation`), so the cost is
 * one tessellation of the leaned-on object per edit, paid only on a lean.
 * When flatness cannot be read the face counts as curved (fail closed).
 */
export class FaceLoopCache {
  private gen: string | null = null
  private loops = new Map<string, V3[] | null>()
  private normals: { object: bigint; normals: Float32Array } | null = null

  /** The world-space outer loop of `face`, or null when it is curved,
   *  stale, degenerate, or its instance has no pose. */
  loopFor(wasmScene: WasmScene, face: HoverFaceRef): V3[] | null {
    const gen = typeof wasmScene.history_generation === 'function' ? String(wasmScene.history_generation()) : ''
    if (gen !== this.gen) {
      this.gen = gen
      this.loops.clear()
      this.normals = null
    }
    const key = `${face.object}:${face.face}`
    let local = this.loops.get(key)
    if (local === undefined) {
      local = this.localLoop(wasmScene, face.object, face.face)
      this.loops.set(key, local)
    }
    if (local === null) return null
    if (face.instance === null) return local
    const pose = wasmScene.instance_pose(face.instance)
    if (pose === undefined) return null
    return local.map((p) => applyAffine3x4(pose, p))
  }

  clear(): void {
    this.gen = null
    this.loops.clear()
    this.normals = null
  }

  private localLoop(wasmScene: WasmScene, object: bigint, face: bigint): V3[] | null {
    if (typeof wasmScene.face_boundary !== 'function') return null
    let flat: ArrayLike<number>
    try {
      flat = wasmScene.face_boundary(object, face)
    } catch {
      return null // a stale handle: nothing to outline
    }
    if (flat.length < 9) return null
    if (this.isCurved(wasmScene, object, face)) return null
    const out: V3[] = []
    for (let i = 0; i + 2 < flat.length; i += 3) out.push([flat[i], flat[i + 1], flat[i + 2]])
    return out
  }

  /** Whether `face` is a chord facet of a curved wall: the kernel's own
   *  answer (`face_is_curved`), else, for a scene without that accessor,
   *  the tessellated normals. Fails closed: a
   *  face whose flatness cannot be read (no mesh call, a throw, a missing
   *  or short range) counts as curved, so the cue falls back to the small
   *  patch rather than outline a sliver. */
  private isCurved(wasmScene: WasmScene, object: bigint, face: bigint): boolean {
    if (typeof wasmScene.face_is_curved === 'function') {
      try {
        return wasmScene.face_is_curved(object, face)
      } catch {
        return true
      }
    }
    if (typeof wasmScene.object_mesh !== 'function' || typeof wasmScene.face_mesh_range !== 'function') return true
    try {
      if (this.normals === null || this.normals.object !== object) {
        const mesh = wasmScene.object_mesh(object)
        try {
          this.normals = { object, normals: mesh.normals() }
        } finally {
          mesh.free()
        }
      }
      const range = wasmScene.face_mesh_range(object, face)
      if (range === undefined || range.length < 2 || range[1] < 1) return true
      const n = this.normals.normals
      const base = range[0] * 3
      const end = (range[0] + range[1]) * 3
      if (end > n.length) return true
      for (let i = base + 3; i + 2 < end; i += 3) {
        if (
          Math.abs(n[i] - n[base]) > FLAT_FACE_NORMAL_EPS ||
          Math.abs(n[i + 1] - n[base + 1]) > FLAT_FACE_NORMAL_EPS ||
          Math.abs(n[i + 2] - n[base + 2]) > FLAT_FACE_NORMAL_EPS
        ) {
          return true
        }
      }
      return false
    } catch {
      return true
    }
  }
}

/** How long (ms) after the last hover over a custom plane the custom pip
 *  still acts on it (view-die design §3.3: the pointer has to cross the
 *  canvas to reach the pip). */
export const CUSTOM_CANDIDATE_MS = 2000

/** The custom pip's candidate plane: where the last custom hover was, when,
 *  and the document state it belongs to. */
interface CustomCandidate {
  plane: DrawPlane
  through: V3
  at: number
  /** `candidateKey` when set: a later edit, undo, axes move or session
   *  change makes it stale. */
  key: string
  /** The pointer left the canvas while it was live: no timeout until the
   *  pointer's next canvas move. */
  latched: boolean
}

/** The document's drawing axes, or world identity for a host that cannot
 *  answer (a test double without `axes`). */
function drawingAxesOf(wasmScene: WasmScene): DrawingAxes {
  return typeof wasmScene.axes === 'function' ? getDrawingAxes(wasmScene) : WORLD_DRAWING_AXES
}

function candidateKey(wasmScene: WasmScene, frame: DrawingAxes): string {
  const gen = typeof wasmScene.history_generation === 'function' ? String(wasmScene.history_generation()) : ''
  // The open session frames: entering or leaving one changes what a click
  // can draw on, whether or not the kernel counts it as an edit.
  let sessions = ''
  if (typeof wasmScene.session_stack === 'function') {
    for (const node of wasmScene.session_stack()) {
      sessions += `${node.kind}:${node.id};`
      node.free()
    }
  }
  return `${gen}|${sessions}|${frame.origin.join(',')}|${frame.x.join(',')}|${frame.y.join(',')}`
}

/**
 * What the custom pip acts on for a tool with `pin` and `tracker` — the ONE
 * order both `customPipPlaneFor` and `pinCustomPlane` use: an active pin
 * (a click releases it; the pip is enabled only when the pinned plane is
 * custom), else the live custom candidate, which already covers a custom
 * plane under the cursor and carries every invalidation (the 2 s window,
 * an edit, undo or redo, an axes move, a session change). The tracked
 * hover (`PlanePin.tracked`) is deliberately not consulted: it has none of
 * those checks.
 */
function customPipTarget(
  wasmScene: WasmScene,
  pin: PlanePin,
  tracker: DrawPlaneTracker,
): { release: true; plane: DrawPlane } | { release: false; target: PinnedPlane } | null {
  const pinned = pin.current
  if (pinned !== null) return planePip(pinned.plane.normal, drawingAxesOf(wasmScene)) === 'custom' ? { release: true, plane: pinned.plane } : null
  const candidate = tracker.customCandidate(wasmScene)
  return candidate === null ? null : { release: false, target: candidate }
}

/**
 * The plane the custom pip acts on, or null (the pip is disabled): a
 * pinned custom plane, else the live custom candidate (`customPipTarget`).
 * Callers return null mid-gesture themselves.
 */
export function customPipPlaneFor(wasmScene: WasmScene, pin: PlanePin, tracker: DrawPlaneTracker): DrawPlane | null {
  const t = customPipTarget(wasmScene, pin, tracker)
  return t === null ? null : t.release ? t.plane : t.target.plane
}

/**
 * The custom pip's click: release the pin, else pin the live candidate (the
 * last custom plane hovered, which the tracked hover may have moved on
 * from), in `customPipTarget`'s order. Returns whether the pin changed.
 * Shift keeps pinning the tracked hover (`PlanePin.setShiftHeld`).
 */
export function pinCustomPlane(wasmScene: WasmScene, pin: PlanePin, tracker: DrawPlaneTracker): boolean {
  if (pin.current !== null) return pin.toggle()
  const t = customPipTarget(wasmScene, pin, tracker)
  if (t === null || t.release) return false
  pin.pinPlane(t.target)
  return true
}

/** The last idle hover a draw tool resolved while no lock or pin decided
 *  the plane. `inert` marks a hover inside an entered object over anything
 *  but that object's faces, where a click does nothing. */
interface IdleHoverTrack {
  plane: DrawPlane
  kind: DrawPlaneHoverKind
  point: V3
  faceLoop?: V3[]
  inert: boolean
}

/**
 * The per-tool bookkeeping behind `Tool.drawPlaneState()` (view-die design
 * §3.4), shared by the five draw tools. Each tool feeds it the cursor ray on
 * every pointer move (`move`), the idle hover's resolution whenever no lock
 * or pin decided the plane (`trackIdle`), and `leave` when the pointer
 * leaves the canvas; `state` then folds in the tool's own gesture, lock and
 * pin. The last idle hover survives `leave`, so an inferred plane stays on
 * the pips while the pointer travels to them.
 */
export class DrawPlaneTracker {
  private idle: IdleHoverTrack | null = null
  private ray: Ray | null = null
  private readonly loops = new FaceLoopCache()
  private candidate: CustomCandidate | null = null

  /** `now` is the clock the custom candidate's window runs on (ms). */
  constructor(private readonly now: () => number = () => performance.now()) {}

  /** The cursor ray of a pointer move (any stage). The first move back on
   *  the canvas ends a custom candidate's latch: its window counts from its
   *  own last hover again. */
  move(ray: Ray): void {
    if (this.ray === null && this.candidate !== null) this.candidate.latched = false
    this.ray = ray
  }

  /** The pointer left the canvas: no hover until the next move. A custom
   *  candidate still live now (under the cursor, or within its window)
   *  stays live, with no timeout, until the pointer comes back (it is on
   *  its way to the custom pip); one already expired is dropped. */
  leave(): void {
    this.endCandidateHover()
    // The tracked plane stays for the pips; it is no longer a live hover.
    if (this.idle !== null) this.idle = { ...this.idle, inert: true }
    this.ray = null
    const c = this.candidate
    if (c !== null) {
      if (c.latched || this.now() - c.at <= CUSTOM_CANDIDATE_MS) c.latched = true
      else this.candidate = null
    }
  }

  /** Whether the cursor is over the candidate's plane right now: the live
   *  idle hover is the very plane the candidate was taken from. */
  private hoveringCandidate(): boolean {
    const c = this.candidate
    const idle = this.idle
    return c !== null && this.ray !== null && idle !== null && !idle.inert && idle.plane === c.plane
  }

  /** The hover over the candidate's plane is ending (the cursor moves on,
   *  resolves to nothing, or leaves): its window counts from now. */
  private endCandidateHover(): void {
    if (this.hoveringCandidate()) this.candidate!.at = this.now()
  }

  /** Record an idle hover the picker resolved without a lock or pin.
   *  `snapPoint` stands in for the ray's hit when the ray misses the plane.
   *  `inObjectContext`: inside an entered object only its faces take a
   *  click, so anything else there is no hover at all. */
  trackIdle(wasmScene: WasmScene, resolution: IdleHoverResolution, ray: Ray, snapPoint: V3, inObjectContext: boolean): void {
    this.ray = ray
    const kind = idleHoverKind(resolution)
    const point = pointOnPlane(ray, resolution.plane) ?? snapPoint
    const track: IdleHoverTrack = {
      plane: resolution.plane,
      kind,
      point,
      inert: inObjectContext && resolution.face === undefined && resolution.onFace !== true,
    }
    if (kind === 'lean' && resolution.face !== undefined) {
      const loop = this.loops.loopFor(wasmScene, resolution.face)
      if (loop !== null) track.faceLoop = loop
    }
    this.endCandidateHover()
    this.idle = track
    // A custom plane the next click would use becomes the custom pip's
    // candidate; any other hover leaves the candidate alone, so the pointer
    // can cross the ground or empty space on its way to the pip.
    if (!track.inert) {
      const frame = drawingAxesOf(wasmScene)
      if (planePip(track.plane.normal, frame) === 'custom') {
        this.candidate = { plane: track.plane, through: point, at: this.now(), key: candidateKey(wasmScene, frame), latched: false }
      }
    }
  }

  /**
   * The custom pip's candidate (view-die design §3.3), or null: the last
   * custom plane an idle hover tracked, live while the cursor is over it
   * and for `CUSTOM_CANDIDATE_MS` after that hover ends, and without a
   * timeout while the pointer is off the canvas if it was live when the
   * pointer left. Dead once the document, its drawing axes or the open
   * session changed since.
   */
  customCandidate(wasmScene: WasmScene): PinnedPlane | null {
    const c = this.candidate
    if (c === null) return null
    if (c.key !== candidateKey(wasmScene, drawingAxesOf(wasmScene))) return null
    if (!c.latched && !this.hoveringCandidate() && this.now() - c.at > CUSTOM_CANDIDATE_MS) return null
    return { plane: c.plane, through: c.through }
  }

  /** Milliseconds until a live, unlatched candidate expires, or null when
   *  there is no timed expiry to wait for (the host schedules one refresh
   *  then, so a resting pointer does not leave the pip looking enabled). */
  customCandidateTtl(): number | null {
    const c = this.candidate
    if (c === null || c.latched || this.hoveringCandidate()) return null
    const left = c.at + CUSTOM_CANDIDATE_MS - this.now()
    return left >= 0 ? left : null
  }

  /** Drop the custom candidate: a pin made or released, an arrow lock set,
   *  a gesture started, a session entered or left. */
  clearCandidate(): void {
    this.candidate = null
  }

  /** The idle hover resolved to nothing this time (no snap): keep the
   *  tracked plane for the pips but drop the hover, so no cue lingers at
   *  the last point. A `trackIdle` in the same event replaces it. */
  clearHover(): void {
    this.endCandidateHover()
    if (this.idle !== null) this.idle = { ...this.idle, inert: true }
  }

  /** Forget the idle hover and the custom candidate (a cancel, a context
   *  change, a tool switch). */
  clearIdle(): void {
    this.idle = null
    this.candidate = null
  }

  /** Drop everything, including the face-loop memo (a new document). */
  reset(): void {
    this.idle = null
    this.ray = null
    this.candidate = null
    this.loops.clear()
  }

  /**
   * The tool's `DrawPlaneState` from its own stage: `anchored`, the plane a
   * gesture under way uses (null when idle); `lock`, the arrow-key plane
   * lock; `pinned`, the Shift-pinned plane; `lockHover`, the last idle hover
   * point (the lock's plane passes through it); `fallback`, the plane an
   * idle click would use before any hover was tracked (the view's inferred
   * plane, or the ground); `frame`, the current drawing axes.
   *
   * Hold: a lock or pin outranks the gesture, since both survive it and a
   * pip click or key releases them (`locked` and `pinned` show a padlock);
   * an unlocked gesture is `anchored`; otherwise `inferred`.
   */
  state(p: {
    anchored: DrawPlane | null
    lock: 0 | 1 | 2 | null
    pinned: DrawPlane | null
    lockHover: V3 | null
    fallback: DrawPlane
    frame: DrawingAxes
  }): DrawPlaneState {
    const ray = this.ray
    const held = (plane: DrawPlane, at: V3 | null): DrawPlaneState['hover'] => {
      if (ray === null) return null
      const point = pointOnPlane(ray, plane) ?? at
      return point === null ? null : { kind: 'space', point }
    }
    let plane: DrawPlane
    let hold: DrawPlaneState['hold']
    let hover: DrawPlaneState['hover']
    if (p.anchored !== null) {
      plane = p.anchored
      hold = p.lock !== null ? 'locked' : p.pinned !== null ? 'pinned' : 'anchored'
      hover = held(plane, null)
    } else if (p.pinned !== null) {
      plane = p.pinned
      hold = 'pinned'
      hover = held(plane, p.lockHover)
    } else if (p.lock !== null) {
      plane = axisDrawPlane(p.lock, p.lockHover ?? [p.frame.origin[0], p.frame.origin[1], p.frame.origin[2]], p.frame)
      hold = 'locked'
      hover = ray === null || p.lockHover === null ? null : { kind: 'space', point: p.lockHover }
    } else {
      const idle = this.idle
      plane = idle?.plane ?? p.fallback
      hold = 'inferred'
      hover = ray === null || idle === null || idle.inert
        ? null
        : idle.faceLoop === undefined
          ? { kind: idle.kind, point: idle.point }
          : { kind: idle.kind, point: idle.point, faceLoop: idle.faceLoop }
    }
    return { plane, pip: planePip(plane.normal, p.frame), hold, hover }
  }
}

/** A face-mode gesture's frozen plane (its plane point and unit normal) as
 *  a `DrawPlane`, or null for a degenerate normal; `ground` set exactly when
 *  the face lies on the ground plane. */
export function faceStageDrawPlane(planePoint: V3, normal: V3): DrawPlane | null {
  const basis = facePlaneBasis(normal)
  if (basis === null) return null
  return { origin: planePoint, normal, u: basis.u, v: basis.v, ground: isGroundPlane(planePoint, normal) }
}

/** The plane an idle draw tool reports before any hover was tracked: the
 *  axis plane the view faces (`inferredAxisPlane`) once a camera has been
 *  fed, else the ground. */
export function idleFallbackPlane(frame: DrawingAxes, view: CameraView | null): DrawPlane {
  return view === null ? groundDrawPlane() : inferredAxisPlane(frame, view)
}

/**
 * The plane a Shift pin reports while idle (`DrawPlaneState`'s pinned
 * plane): `pinned` itself, or, when the hover point `through` sits off it,
 * `pinned`'s orientation through that point. A pin holds the plane's
 * orientation, not its height: under a pin the cursor stays on the plane
 * except at a named point off it (`PlanePin.constraint`'s `offPlanePoints`),
 * where a first click starts the shape at that point (`PlanePin.pointPlane`),
 * so the state shows that lifted plane, and a ground pin lifted off the
 * ground is no longer the ground.
 */
export function pinnedPlaneThrough(pinned: DrawPlane, through: V3 | null): DrawPlane {
  if (through === null) return pinned
  if (Math.abs(distanceOffPlane(through, pinned.origin, pinned.normal)) <= FACE_PLANE_EPS_M) return pinned
  return { ...pinned, origin: through, ground: isGroundPlane(through, pinned.normal) }
}
