/**
 * faceDraw — the shared "may this face be drawn on directly?" policy for the
 * draw tools (Line / Rectangle / Circle / Polygon / Arc), plus the per-pointer-event
 * pick cache they all use to avoid re-running the O(faces) `pick_face`
 * raycast two or three times for the same ray (snapConstraint and the
 * pointer-move/down dispatcher are called back-to-back by the Viewport).
 *
 * Policy ("plain objects are immediately editable"):
 *   - Inside an entered object context: only that object's faces.
 *   - Top level: any PLAIN object's face — ungrouped and not part of a
 *     component instance. Groups and Components keep their explicit
 *     double-click editing step, so a face belonging to instanced geometry
 *     or to an object nested in a group is not directly drawable.
 *
 * The Viewport can inject a richer predicate (via each tool's
 * `setFaceEligibility`) that also understands group/instance editing
 * contexts — the tools themselves only know the entered-object id.
 *
 * What happens on an INELIGIBLE face differs by tool class, deliberately:
 *
 * - The DRAW tools draw on that face's WORLD plane as free drawing, the way
 *   SketchUp draws loose geometry on a group's or component's face from
 *   outside: the cursor reads "On Face", the draft commits in the plane's
 *   context (world, no instance, no object), and the ink lands sitting on
 *   the face — not inside the group/component and not on the ground beneath
 *   it (`ineligibleFacePlane`, consumed by `resolveIdleDrawTarget`). This
 *   applies at the top level only; inside an editing context the tools
 *   keep their scoped rules. Faces of plain objects are eligible and are
 *   drawn on directly, as before. Pinned in RectangleTool.test.ts /
 *   LineTool.test.ts ("grouped → free drawing on the face's plane").
 * - PUSH/PULL instead CONSUMES the click with an explanatory toast
 *   (PushPullTool). Its fallthrough target would be an existing sketch
 *   region along the same ray — falling through would silently start a
 *   drag on, and then extrude, geometry the user did not aim at. A
 *   mutation of existing geometry must fail closed where new, previewed
 *   ink may fall through.
 */

import { t } from '@lingui/core/macro'
import type { Ray } from '../viewport/math'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { applyAffine3x4, rayPlaneIntersect, transformNormalThroughPose } from '../viewport/geoHelpers'
import { parseKernelErrorCode } from '../kernelErrors'
import { pickPolicyGeneration, pickRenderMode } from './pickPolicy'

/** May the face on `object` (hit through `instance`, when the ray struck
 *  instanced geometry) be drawn on directly? */
export type FaceEligible = (object: bigint, instance: bigint | undefined) => boolean

/** An eligible face under the cursor: the pick's object + face handles. */
export interface EligibleFacePick {
  object: bigint
  face: bigint
}

/** The raw `pick_face` result under the cursor, regardless of eligibility —
 *  `object`/`instance`/`face` exactly as the WASM pick reported them before
 *  `FacePickCache.pickFor`'s eligibility filter ran, plus the hit's ray
 *  `depth` (metres along the normalized ray). Lets a caller that needs the
 *  INELIGIBLE case too (PushPullTool's onPointerDown: the toast hint names
 *  the instance the rejected face belonged to) read it back from the SAME
 *  memoized pick instead of re-raycasting for it, and lets a caller with a
 *  competing pick (PushPullTool's sketch-region pick) compare depths so a
 *  drawn region IN FRONT of the face wins the hover, not the face behind it. */
export interface RawFacePick {
  object: bigint
  instance: bigint | undefined
  face: bigint
  depth: number
}

/**
 * The WORLD-space unit normal of `object`'s `face` — the mapping every draw
 * tool (Line/Rectangle/Circle/Polygon/Arc) and PushPullTool need before
 * using `face_normal`'s result in world-space math (`rayPlaneIntersect`,
 * the push/pull drag axis, …).
 *
 * `face_normal` is documented (`crates/wasm-api/src/lib.rs`) as answering in
 * the Object's OWN LOCAL frame — which equals world space for a world
 * object (identity-placed) or a Group member (Groups bake every transform
 * into member geometry, never carry their own pose), but NOT for a
 * component INSTANCE's member: an instance carries a real, un-baked pose
 * that may rotate, mirror, or non-uniformly scale (component-edit-parity.md
 * phase A2 — the first time these tools ever draw on a face inside such a
 * context). `activeInstance` is the tool's own entered-instance id (null at
 * top level, inside an object/group context, or when `object` isn't reached
 * through an instance at all) — when set, the local normal is posed forward
 * through `activeInstance`'s pose via the inverse-transpose (`
 * transformNormalThroughPose`), never the plain linear part, which would
 * silently mis-tilt the result under any non-uniform scale.
 *
 * Returns `null` for a stale instance or a degenerate/singular pose —
 * callers should treat that exactly like a missing/stale face pick (stay
 * idle), never fall back to the raw local normal.
 */
export function worldFaceNormal(
  wasmScene: WasmScene,
  object: bigint,
  face: bigint,
  activeInstance: bigint | null,
): V3 | null {
  let normalArr: ReturnType<WasmScene['face_normal']>
  try {
    normalArr = wasmScene.face_normal(object, face)
  } catch (err) {
    if (isStaleHandle(err)) return null
    throw err
  }
  const local: V3 = [normalArr[0], normalArr[1], normalArr[2]]
  if (activeInstance === null) return local
  const pose = wasmScene.instance_pose(activeInstance)
  if (pose === undefined) return null
  return transformNormalThroughPose(pose, local)
}

/**
 * Whether a kernel throw says the handle names nothing any more — the one
 * failure `worldFaceNormal`/`worldFacePlane` promise to answer with `null`.
 * A memoized face pick can outlive its face between pointer events: a
 * typed retype undoes and recommits the imprint under a stationary cursor,
 * and the next key press re-resolves the snap against the cached ray, so
 * the pick still names the sub-face the undo removed. Any other kernel
 * error is a real fault and still propagates.
 */
function isStaleHandle(err: unknown): boolean {
  const code = parseKernelErrorCode(err)
  return code === 'UnknownObject' || code === 'UnknownFace'
}

/** Metres off a face's plane a point may sit and still count as ON that
 *  face for every face-adoption test in the draw tools (`FacePickCache.
 *  faceThrough`, LineTool's re-adoption) — the kernel's own plane tolerance
 *  scale (`GROUND_PLANE_EPS`), well below any drawn feature. */
export const FACE_PLANE_EPS_M = 1e-6

/** A face's world-space plane: a point on it and its unit normal, mapped
 *  through the active instance's pose when drawing inside one (the same
 *  mapping `worldFaceNormal` applies to the normal). `null` for a stale
 *  handle or a singular pose — callers treat that exactly like a miss. */
export function worldFacePlane(
  wasmScene: WasmScene,
  object: bigint,
  face: bigint,
  activeInstance: bigint | null,
): { point: V3; normal: V3 } | null {
  const normal = worldFaceNormal(wasmScene, object, face, activeInstance)
  if (normal === null) return null
  let planeArr: ReturnType<WasmScene['face_plane']>
  try {
    planeArr = wasmScene.face_plane(object, face)
  } catch (err) {
    if (isStaleHandle(err)) return null
    throw err
  }
  let point: V3 = [planeArr[0], planeArr[1], planeArr[2]]
  if (activeInstance !== null) {
    const pose = wasmScene.instance_pose(activeInstance)
    if (pose === undefined) return null
    point = applyAffine3x4(pose, point)
  }
  return { point, normal }
}

/** Whether `snap` sits on an OBJECT's edge or corner — the one place a pick
 *  ray misses the faces under it (strict polygon test at the boundary), so
 *  the ranked boundary pick (`FacePickCache.faceThrough`) must answer which
 *  face the point belongs to. A type guard: the snap is non-null past it. */
export function snapOnObjectBoundary(
  snap: Snap | null,
): snap is Snap & { object: bigint; elementKind: 'vertex' | 'edge' } {
  return (
    snap !== null &&
    snap.object !== undefined &&
    (snap.elementKind === 'vertex' || snap.elementKind === 'edge')
  )
}

/**
 * Where a cursor lands on a draw tool's frozen plane (a face, or a
 * non-ground sketch plane) that must stay planar: the snapped point when it
 * lies on the plane, else — a snap OFF the plane, which a non-occluding
 * render mode (X-ray, Wireframe) offers through the face the tool is held
 * to — that point dropped straight onto the plane, with `projected` set so
 * the cue says so (`Tool.snapProjected`), exactly as the ground plane drops
 * a snap's height. With no snap, the ray's hit on the plane (`null` when the
 * ray runs parallel to it).
 */
export function cursorOnFrozenPlane(
  snap: Snap | null,
  ray: Ray,
  planePoint: V3,
  normal: V3,
): { point: V3; projected: boolean } | null {
  if (snap !== null) {
    const p: V3 = [snap.x, snap.y, snap.z]
    const off = distanceOffPlane(p, planePoint, normal)
    if (Math.abs(off) <= FACE_PLANE_EPS_M) return { point: p, projected: false }
    const n2 = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]
    if (n2 > 0) {
      const k = off / n2
      return { point: [p[0] - normal[0] * k, p[1] - normal[1] * k, p[2] - normal[2] * k], projected: true }
    }
  }
  const hit = rayPlaneIntersect(ray.origin, ray.direction, planePoint, normal)
  return hit === null ? null : { point: hit, projected: false }
}

/**
 * The first point of a gesture started on `face` (the hovered or adopted
 * face) from `snap`: the snap itself when it lies on the face's plane — any
 * visible snap, and a hidden edge or corner whose face `faceThrough`
 * adopted in X-ray — else, for a hidden point that is no part of the face
 * (an edge crossing, a drawn center), that point dropped onto the face's
 * plane and marked `projected`, exactly as the gesture's later clicks are
 * (`cursorOnFrozenPlane`). Without it such a point anchored a plane
 * parallel to the face but off it by the point's depth.
 */
export function firstPointOnFace(
  wasmScene: WasmScene,
  face: { object: bigint; face: bigint; normal: V3 },
  activeInstance: bigint | null,
  snap: Snap,
  ray: Ray,
): { point: V3; projected: boolean } {
  const raw: V3 = [snap.x, snap.y, snap.z]
  const plane = worldFacePlane(wasmScene, face.object, face.face, activeInstance)
  if (plane === null) return { point: raw, projected: false }
  return cursorOnFrozenPlane(snap, ray, plane.point, face.normal) ?? { point: raw, projected: false }
}

/** Signed distance of `p` off the plane through `point` with unit `normal`. */
export function distanceOffPlane(p: V3, point: V3, normal: V3): number {
  return normal[0] * (p[0] - point[0]) + normal[1] * (p[1] - point[1]) + normal[2] * (p[2] - point[2])
}

/** The toast code the face-editing tools attach to an `ineligibleFaceHint`
 *  refusal. Not a kernel error: it marks the toast as GUIDANCE (how to get
 *  in — double-click the component, enter the group, step out) so the app
 *  can show it more prominently and keep it up longer than a plain warning,
 *  without ever classifying it as an error. The hint is easy to miss as an
 *  ordinary bottom toast (bug report: a person coming from SketchUp pushed
 *  on a component face, saw nothing happen, and read the viewport as
 *  broken). */
// oxlint-disable-next-line lingui/no-unlocalized-strings -- a toast code, not copy
export const EDIT_CONTEXT_HINT_CODE = 'EditContextHint'

/** Why an ineligible face refused, phrased as the way in — shared by every
 *  tool that fails closed on one (Push/Pull, Offset). Inside ANY editing
 *  context the refusal is the scope ('step out' is the only honest
 *  guidance); at the top level an instanced pick belongs to a component and
 *  anything else was a grouped face. `verb` names the tool's action. */
export function ineligibleFaceHint(
  verb: string,
  contextScoped: boolean,
  instance: bigint | undefined,
  sessionOpen = false,
): string {
  if (contextScoped) {
    return t`${verb} is scoped to what you are editing — press Esc to step out first`
  }
  // A group or component is open for editing (a session frame; the tool's
  // edit context stays at the top level for one). Double-clicking a face
  // outside it does nothing until the session is finished, so "double-click
  // it" would send the person down a dead end — name the real way out.
  if (sessionOpen) {
    return instance !== undefined
      ? t`That face belongs to another copy — press Esc to finish editing the open component first, then double-click this one`
      : t`That face isn't part of what you are editing — press Esc to finish first`
  }
  if (instance !== undefined) {
    return t`That face is part of a component — double-click it to edit the component`
  }
  return t`That face is inside a group — double-click to enter the group and edit it`
}

/** Whether a group or component editing session is open (`session_stack`),
 *  for `ineligibleFaceHint`'s wording. */
export function sessionIsOpen(wasmScene: WasmScene): boolean {
  return wasmScene.session_stack().length > 0
}

/**
 * The default tool-local policy (used when the Viewport hasn't injected one):
 * scoped to the entered object inside a context; at top level, plain
 * (ungrouped, non-instanced) objects only.
 */
export function defaultFaceEligible(
  wasmScene: WasmScene,
  activeContext: bigint | null,
  object: bigint,
  instance: bigint | undefined,
): boolean {
  if (activeContext !== null) {
    return instance === undefined && object === activeContext
  }
  if (instance !== undefined) return false
  // kind 0 = object; a defined parent means it lives inside a group.
  return wasmScene.node_parent(0, object) === undefined
}

/** An INELIGIBLE face under the cursor — a face of a group member or a
 *  component instance at the top level — as the world plane the draw tools
 *  draw on instead: a point on it, its WORLD unit normal, and the hit's
 *  depth along the ray. */
export interface IneligibleFacePlane {
  point: V3
  normal: V3
  depth: number
}

/** The world plane of the face under `ray` when it was hit but is NOT
 *  eligible for direct drawing (`null` for a miss, an eligible face, or a
 *  stale handle). Uses the same memoized pick as `FacePickCache.pickFor`;
 *  instanced faces map through the instance the ray struck them through. */
export function ineligibleFacePlane(
  wasmScene: WasmScene,
  cache: FacePickCache,
  ray: Ray,
  isEligible: FaceEligible,
  snap: Snap | null = null,
): IneligibleFacePlane | null {
  // A click ON an ineligible object's edge or corner: the plain ray misses
  // there, so the ranked boundary pick answers, over INELIGIBLE faces only.
  // An eligible face holding the snapped point wins first (its tool takes
  // face mode before ever asking for this).
  if (snapOnObjectBoundary(snap)) {
    const p: V3 = [snap.x, snap.y, snap.z]
    if (cache.faceThrough(wasmScene, ray, isEligible, null, [p]) !== null) return null
    const through = cache.faceThrough(wasmScene, ray, isEligible, null, [p], true)
    if (through !== null) {
      return { point: through.point, normal: through.normal, depth: Math.hypot(p[0] - ray.origin[0], p[1] - ray.origin[1], p[2] - ray.origin[2]) }
    }
  }
  if (cache.pickFor(wasmScene, ray, isEligible) !== null) return null
  const raw = cache.rawPickFor(ray)
  if (raw === null) return null
  const plane = worldFacePlane(wasmScene, raw.object, raw.face, raw.instance ?? null)
  if (plane === null) return null
  return { point: plane.point, normal: plane.normal, depth: raw.depth }
}

/** Angular nudge of the boundary probes in `FacePickCache.faceThrough` —
 *  about a pixel and a half of a 45° view on a 720 px canvas, so a probe
 *  lands on the face beside an edge without reaching anything a hand would
 *  not have aimed at. */
const PICK_NUDGE_RAD = 0.002

/** Eight directions a hair off `dir`, evenly spaced around it: the four
 *  screen-axis nudges (up, down, left, right) and the four diagonals. A ray
 *  through a point ON an edge or corner misses both faces that meet there
 *  (the polygon test is strict at the boundary), and a single nudge can miss
 *  too — sliding along a vertical edge, say — so the probe is a full ring. */
function ringDirections(dir: V3): V3[] {
  const [dx, dy, dz] = dir
  const len = Math.hypot(dx, dy, dz)
  if (len < 1e-12) return []
  const d: V3 = [dx / len, dy / len, dz / len]
  // A perpendicular pair: `u` sideways (horizontal when possible), `v` the
  // remaining axis (roughly screen-vertical).
  const upRef: V3 = Math.abs(d[2]) < 0.99 ? [0, 0, 1] : [0, 1, 0]
  let u: V3 = [d[1] * upRef[2] - d[2] * upRef[1], d[2] * upRef[0] - d[0] * upRef[2], d[0] * upRef[1] - d[1] * upRef[0]]
  const ul = Math.hypot(u[0], u[1], u[2])
  u = [u[0] / ul, u[1] / ul, u[2] / ul]
  const v: V3 = [u[1] * d[2] - u[2] * d[1], u[2] * d[0] - u[0] * d[2], u[0] * d[1] - u[1] * d[0]]
  const e = PICK_NUDGE_RAD
  const out: V3[] = []
  for (let k = 0; k < 8; k++) {
    const a = (k * Math.PI) / 4
    const cu = Math.cos(a) * e
    const sv = Math.sin(a) * e
    out.push([d[0] + u[0] * cu + v[0] * sv, d[1] + u[1] * cu + v[1] * sv, d[2] + u[2] * cu + v[2] * sv])
  }
  return out
}

/** An eligible face found by `FacePickCache.faceThrough`, with its world
 *  plane (already mapped through the active instance's pose). */
export interface FaceThroughPick extends EligibleFacePick {
  point: V3
  normal: V3
}

/**
 * Memoizes the single `pick_face` raycast for the CURRENT pointer event.
 * Keyed by reference equality on the `Ray` passed in (the Viewport builds one
 * Ray object per event); a miss just falls back to a fresh pick.
 * `eligible: null` means either nothing was hit, or a face was hit but the
 * eligibility predicate rejected it.
 */
export class FacePickCache {
  private cache: { ray: Ray; gen: number; raw: RawFacePick | null; eligible: EligibleFacePick | null } | null = null
  private throughCache: { ray: Ray; gen: number; key: string; pick: FaceThroughPick | null } | null = null

  pickFor(
    wasmScene: WasmScene,
    ray: Ray,
    isEligible: FaceEligible,
  ): EligibleFacePick | null {
    if (this.cache !== null && this.cache.ray === ray && this.cache.gen === pickPolicyGeneration()) {
      return this.cache.eligible
    }
    const raw = rawPick(wasmScene, ray.origin, ray.direction)
    let eligible: EligibleFacePick | null = null
    if (raw !== null && isEligible(raw.object, raw.instance)) {
      eligible = { object: raw.object, face: raw.face }
    }
    this.cache = { ray, gen: pickPolicyGeneration(), raw, eligible }
    return eligible
  }

  /**
   * The eligible face under `ray` whose plane holds every point in
   * `through` — the answer to "which face did a click ON an edge or corner
   * mean?", where the plain `pickFor` is blind: a ray through a point on
   * the boundary misses both faces meeting there (strict polygon test), and
   * even a ray a sub-pixel to one side lands on whichever neighbour the
   * rounding favours — a coin toss the user cannot see, and one that put a
   * Rectangle started on a vertical edge's midpoint onto the ground plane.
   *
   * Candidates are the plain ray hit plus a ring of eight probes a hair
   * around it (`ringDirections`), deduplicated, eligibility-filtered, and
   * kept only when their plane contains every `through` point within
   * `FACE_PLANE_EPS_M` (the snapped point itself; for a Line chain's second
   * point, the anchor as well — the face the user is drawing ACROSS holds
   * both). Ties between the surviving neighbours — a vertical edge shared by
   * two visible walls — go to the face most squarely facing the camera
   * (largest `normal · −ray`): the one the user sees most of, and the one a
   * SketchUp user expects the edge click to land on. `null` when no
   * eligible face through those points is under or beside the cursor.
   *
   * Memoized per `ray` reference AND `through` set, like `pickFor`; a
   * caller pairs it with the snap under the cursor (an object vertex or
   * edge — see LineTool's `_snapOnBoundary`), never with a free point, so
   * the nine raycasts run only for boundary hovers/clicks.
   */
  faceThrough(
    wasmScene: WasmScene,
    ray: Ray,
    isEligible: FaceEligible,
    activeInstance: bigint | null,
    through: readonly V3[],
    ineligible = false,
  ): FaceThroughPick | null {
    const key = through.map((p) => p.join(',')).join(';') + `|${activeInstance ?? ''}|${ineligible ? 'x' : ''}`
    if (this.throughCache !== null && this.throughCache.ray === ray && this.throughCache.gen === pickPolicyGeneration() && this.throughCache.key === key) {
      return this.throughCache.pick
    }
    const seen = new Set<string>()
    const candidates: { pick: FaceThroughPick; facing: number }[] = []
    const dirLen = Math.hypot(ray.direction[0], ray.direction[1], ray.direction[2])
    const consider = (raw: RawFacePick | null) => {
      if (raw === null) return
      const id = `${raw.object}:${raw.face}:${raw.instance ?? ''}`
      if (seen.has(id)) return
      seen.add(id)
      if (isEligible(raw.object, raw.instance) === ineligible) return
      const plane = worldFacePlane(wasmScene, raw.object, raw.face, ineligible ? (raw.instance ?? null) : activeInstance)
      if (plane === null) return
      for (const p of through) {
        if (Math.abs(distanceOffPlane(p, plane.point, plane.normal)) > FACE_PLANE_EPS_M) return
      }
      const facing = dirLen < 1e-12
        ? 0
        : -(plane.normal[0] * ray.direction[0] + plane.normal[1] * ray.direction[1] + plane.normal[2] * ray.direction[2]) / dirLen
      candidates.push({ pick: { object: raw.object, face: raw.face, point: plane.point, normal: plane.normal }, facing })
    }
    // The plain ray first (reusing `pickFor`'s memo when it already ran for
    // this ray), then the ring.
    if (this.cache !== null && this.cache.ray === ray && this.cache.gen === pickPolicyGeneration()) {
      consider(this.cache.raw)
    } else {
      consider(rawPick(wasmScene, ray.origin, ray.direction))
    }
    for (const d of ringDirections(ray.direction)) {
      consider(rawPick(wasmScene, ray.origin, d))
    }
    // Nothing found, though the point is on an Object's edge or corner: in
    // X-ray it is hidden behind nearer faces, which nothing occludes there.
    // Ask where the geometry is instead — the same probes, started just
    // short of the point so the faces in front are skipped, against the
    // policy-blind pick — so a draw tool started on that point lands on the
    // face it lies on rather than falling through to the ground. Never in
    // Wireframe: faces do not exist for input there, so a gesture started
    // on a point keeps the ground or a locked plane.
    if (candidates.length === 0 && through.length > 0 && pickRenderMode() === 'xray') {
      const p = through[0]
      const dist = Math.hypot(p[0] - ray.origin[0], p[1] - ray.origin[1], p[2] - ray.origin[2])
      const start = dist * (1 - THROUGH_RESTART_FRACTION)
      for (const d of [ray.direction, ...ringDirections(ray.direction)]) {
        const len = Math.hypot(d[0], d[1], d[2])
        if (len < 1e-12) continue
        const origin: V3 = [ray.origin[0] + (d[0] / len) * start, ray.origin[1] + (d[1] / len) * start, ray.origin[2] + (d[2] / len) * start]
        consider(rawGeometryPick(wasmScene, origin, d))
      }
    }
    // Most camera-facing first; a tie keeps probe order (plain ray, then
    // the ring from screen-right counter-clockwise), which is deterministic.
    candidates.sort((a, b) => b.facing - a.facing)
    const pick = candidates.length > 0 ? candidates[0].pick : null
    this.throughCache = { ray, gen: pickPolicyGeneration(), key, pick }
    return pick
  }

  /**
   * The raw pick for the exact `ray` `pickFor` was last called with —
   * `object`/`instance`/`face` regardless of eligibility — so a caller that
   * also needs the INELIGIBLE case (PushPullTool.onPointerDown's fail-closed
   * toast, which names the rejected face's instance) can read it back from
   * this SAME memoized pick instead of re-raycasting. `null` when `pickFor`
   * hasn't been called for this `ray` yet (a cache miss — the caller must
   * call `pickFor` first) or found no face at all.
   *
   * Distinguishing "no face hit" (`rawPickFor` null after a `pickFor` call
   * for this ray) from "a face was hit but is ineligible" (`rawPickFor`
   * non-null, `pickFor`'s own return null) is exactly what lets a caller
   * fail closed on the latter instead of silently falling through to a
   * fallback target behind the ineligible face — see PushPullTool's
   * `snapConstraint` / `onPointerDown`, which must agree on this.
   */
  rawPickFor(ray: Ray): RawFacePick | null {
    if (this.cache !== null && this.cache.ray === ray && this.cache.gen === pickPolicyGeneration()) {
      return this.cache.raw
    }
    return null
  }

  clear(): void {
    this.cache = null
    this.throughCache = null
  }
}

/** How far short of a hidden boundary point (as a fraction of its distance
 *  from the eye) `FacePickCache.faceThrough`'s fallback probes start: past
 *  the faces in front of it, near enough that the probes' ring still lands
 *  on the faces meeting at it. */
const THROUGH_RESTART_FRACTION = 0.02

/** One `pick_face_geometry` raycast (blind to the render mode's face
 *  policy), with the wasm handle freed; `null` for a miss or a scene double
 *  without it. */
function rawGeometryPick(wasmScene: WasmScene, origin: V3, direction: V3): RawFacePick | null {
  const geometry = (wasmScene as Partial<WasmScene>).pick_face_geometry
  if (geometry === undefined) return null
  const pick = geometry.call(wasmScene, origin[0], origin[1], origin[2], direction[0], direction[1], direction[2])
  if (pick === undefined) return null
  try {
    return { object: pick.object(), instance: pick.instance(), face: pick.face(), depth: pick.depth() }
  } finally {
    pick.free()
  }
}

/** One `pick_face` raycast, with the wasm handle freed. */
function rawPick(wasmScene: WasmScene, origin: V3, direction: V3): RawFacePick | null {
  const pick = wasmScene.pick_face(origin[0], origin[1], origin[2], direction[0], direction[1], direction[2])
  if (pick === undefined) return null
  try {
    return { object: pick.object(), instance: pick.instance(), face: pick.face(), depth: pick.depth() }
  } finally {
    pick.free()
  }
}
