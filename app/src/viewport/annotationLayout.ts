/**
 * annotationLayout — pure geometry shared by `DimensionTool` (live preview)
 * and `SceneRenderer` (committed render) for the playtest-round-2 dimension
 * fixes (`docs/design/dimensions-playtest2.md`):
 *
 *  - §1 view-direction depth bias (`annotationViewBiasVector`) — findings 2/3.
 *  - §2 dimension-line/label gap layout (`computeLineLabelLayout`,
 *    `screenRoomBeyond`, `pickOutsideEnd`) — finding 1.
 *  - §3 the linear-dimension working plane (`axisDimensionPlane`,
 *    `lockedDimensionPlaneNormal`) — finding 4, as CORRECTED by the
 *    angle-dimensions fix (see the §3 section comment: the original
 *    "view-facing" rule was itself the defect).
 *  - §4 radial "drawn geometry must show the measurement"
 *    (`buildRadialGeometry`, `chordPassesNearCentre`) — finding 5.
 *
 * No THREE-scene-graph state lives here — everything is testable with plain
 * vectors/cameras, mirroring `geoHelpers.ts`/`TextBillboard.ts`'s split
 * between pure math and the stateful classes that use it.
 */
import * as THREE from 'three'
import type { V3 } from './geoHelpers'
import { crossV3, dotV3, normalizeV3, perpComponentV3, rayPlaneIntersect, subV3 } from './geoHelpers'

// ---------------------------------------------------------------- §1: depth bias
//
// dimensions-playtest2.md §1's z-fighting fix no longer lives here as a
// world-space nudge. It used to: a per-vertex push toward the camera along
// the view direction, capped at a fixed world-space ceiling
// (`ANNOTATION_BIAS_MAX_WORLD`, `annotationViewBiasVector`, both since
// removed). Two independent problems with that mechanism:
//
//  - Structurally, it has a genuine blind spot at exact edge-on incidence:
//    when the coincident surface's plane contains the view direction, the
//    nudge (toward the camera) has ZERO component perpendicular to that
//    plane, so no cap size can make it move the vertex off the surface —
//    the direction itself is wrong, not just its magnitude, in that
//    degenerate case. (The pre-existing unit test for this actually proved
//    the WRONG property — that the bias vector's overall LENGTH stays
//    nonzero at edge-on — never checking the perpendicular component that
//    is the only one that matters for winning a coincident depth tie.)
//  - Empirically, the shipped regression test for this ("no z-fighting…",
//    `app/e2e/dimensions-depth.spec.ts`) never actually exercised it: it
//    sampled pixels with a stale, previous-pose bias baked into the
//    annotation geometry (`pixelColorAt`'s underlying `captureFrame` does
//    NOT itself run the per-frame annotation-billboard update that applies
//    this bias — a real frame yield is required first). Re-measured with a
//    real yield and a robust reference-pixel choice (the test's own
//    single-point reference could itself land inside the annotation's own
//    label gap), the coincident-face tie was NOT the mechanism's clearest
//    failure — occlusion was: at the reviewer's own ~11.6-world-unit
//    reproduction distance, a genuinely nearer (2mm) occluder failed to
//    occlude, because the 0.3mm world-space nudge is scale-invariant while
//    ordinary perspective foreshortening is not.
//
// The fix now lives on `depthPolicy.ts`'s ladder instead: annotation fat-line
// and label-quad materials carry NO `glPolygonOffset` of their own
// (`DEPTH_BIAS.ANNOTATION`, `SceneRenderer.ts`, deliberately zero) — see
// `depthPolicy.ts`'s doc comment for the full history, including why a
// MODEST (rather than zero) offset was tried and also measured leaking past
// a real occluder, just at a longer distance.

// ---------------------------------------------------------- §2: line/label gap

/** Padding around a broken dimension line's label, as a multiple of the
 * label's own on-screen height (docs/design/dimensions-playtest2.md §2). */
export const LINE_BREAK_PADDING_FRAC = 0.4

export type LineLabelLayout =
  | { mode: 'broken'; gapStart: number; gapEnd: number }
  | { mode: 'outside'; end: 0 | 1; labelCenterT: number }

/**
 * Pure 1D layout for breaking a dimension/leader line around its label
 * (docs/design/dimensions-playtest2.md §2): the label is nominally centered
 * at the line's own midpoint. `lineLen`/the returned t-values are measured
 * along the line from its start (t=0, "end 0") to its end (t=lineLen, "end
 * 1"). When the label (plus padding) is wider than the line, the line is
 * left whole and the label instead moves outside one end — the end
 * `freeSpaceAtEnd` reports more room past (any consistent unit; only the two
 * calls' relative magnitude matters). A tie goes to end 1 — callers arrange
 * end 1 to be the "right/up" end, per the design's tie-break rule.
 */
export function computeLineLabelLayout(
  lineLen: number,
  labelWidth: number,
  labelHeight: number,
  freeSpaceAtEnd: (end: 0 | 1) => number,
): LineLabelLayout {
  const padding = LINE_BREAK_PADDING_FRAC * labelHeight
  const gapHalf = labelWidth / 2 + padding
  if (gapHalf * 2 < lineLen) {
    const mid = lineLen / 2
    return { mode: 'broken', gapStart: mid - gapHalf, gapEnd: mid + gapHalf }
  }
  const room0 = freeSpaceAtEnd(0)
  const room1 = freeSpaceAtEnd(1)
  const end: 0 | 1 = room1 >= room0 ? 1 : 0
  const labelCenterT = end === 1 ? lineLen + gapHalf : -gapHalf
  return { mode: 'outside', end, labelCenterT }
}

function projectToScreenPx(
  camera: THREE.Camera,
  world: THREE.Vector3,
  viewportWidth: number,
  viewportHeight: number,
): { x: number; y: number } {
  const ndc = world.clone().project(camera)
  return { x: (ndc.x * 0.5 + 0.5) * viewportWidth, y: (-ndc.y * 0.5 + 0.5) * viewportHeight }
}

/**
 * True iff `world` sits behind `camera` — its view-space Z is on the wrong
 * side of the lens (three.js cameras look down their own local -Z, so a
 * point genuinely in front has NEGATIVE view-space Z; behind is >= 0).
 *
 * Needed because `Vector3.project`'s NDC output is silent about this: the
 * perspective divide (`x/w`, `y/w`) uses `w`, which flips sign along with
 * the point crossing behind the camera — so a point directly behind can
 * project to an NDC x/y that lands right back inside `[-1, 1]`, reading as
 * "on screen" to a check that only looks at the divided x/y (playtest-2
 * DELTA review finding 3). A point straight behind the camera along its
 * view axis is the sharpest case: it projects to NDC (0, 0) — screen
 * center — indistinguishable from the most on-screen point possible unless
 * this is checked first.
 */
function isBehindCamera(camera: THREE.Camera, world: THREE.Vector3): boolean {
  const viewSpace = world.clone().applyMatrix4(camera.matrixWorldInverse)
  return viewSpace.z >= 0
}

/**
 * Room, in screen pixels, from `origin`'s projection to the viewport
 * boundary when moving in `outwardWorldDir` — the "more free space on
 * screen" measure `computeLineLabelLayout`'s outside-placement choice needs
 * (docs/design/dimensions-playtest2.md §2). A tiny probe step establishes
 * the on-screen direction (perspective distorts world directions
 * non-linearly, so this can't be read off a single linear camera-basis
 * projection); the slab (ray–box) distance to the nearest viewport edge
 * along that on-screen direction is the result.
 *
 * Falls back to the viewport's own diagonal — a neutral "plenty of room"
 * value that never wins a room COMPARISON against a genuinely bounded
 * direction, since both candidates would fall back identically in the
 * degenerate case — when the probe collapses on-screen (near-zero on-screen
 * extent: the camera looking straight down `outwardWorldDir`) or the
 * viewport has no area.
 *
 * Returns 0 — never the fallback, never a ray-to-edge distance — when
 * `origin` itself has already scrolled off the viewport (playtest-2 review
 * finding 2) OR sits behind the camera entirely (playtest-2 DELTA review
 * finding 3 — `isBehindCamera`; a point behind the camera can project to an
 * NDC x/y that reads as on-screen, since the sign flip that puts it behind
 * also flips the perspective divide, so the off-screen bounds check alone
 * cannot catch it). Either way there is no genuine "room to push a label
 * past": the point isn't visible to push FROM, and the ray-to-edge distance
 * below is computed as if it were still inside the box, which for a point
 * already off-screen (or behind the lens) and probing back toward the other
 * edge can read as almost the full viewport span — worse, "plenty of room"
 * exactly where there is none. Zero always loses `computeLineLabelLayout`'s
 * "more room wins" comparison to a genuinely on-screen, bounded candidate,
 * so an invisible end can never out-bid a visible one; the both-invisible
 * case still resolves by the existing tie-break to end 1.
 */
export function screenRoomBeyond(
  camera: THREE.Camera,
  origin: THREE.Vector3,
  outwardWorldDir: THREE.Vector3,
  viewportWidth: number,
  viewportHeight: number,
): number {
  const fallback = Math.hypot(viewportWidth, viewportHeight)
  if (viewportWidth <= 0 || viewportHeight <= 0) return fallback
  if (isBehindCamera(camera, origin)) return 0
  const p0 = projectToScreenPx(camera, origin, viewportWidth, viewportHeight)
  if (p0.x < 0 || p0.x > viewportWidth || p0.y < 0 || p0.y > viewportHeight) return 0
  const dist = camera.position.distanceTo(origin)
  const eps = Math.max(dist, 1) * 0.01
  const p1 = projectToScreenPx(
    camera,
    origin.clone().addScaledVector(outwardWorldDir, eps),
    viewportWidth,
    viewportHeight,
  )
  let dx = p1.x - p0.x
  let dy = p1.y - p0.y
  const len = Math.hypot(dx, dy)
  if (len < 1e-6) return fallback
  dx /= len
  dy /= len
  let t = Infinity
  if (dx > 1e-9) t = Math.min(t, (viewportWidth - p0.x) / dx)
  else if (dx < -1e-9) t = Math.min(t, (0 - p0.x) / dx)
  if (dy > 1e-9) t = Math.min(t, (viewportHeight - p0.y) / dy)
  else if (dy < -1e-9) t = Math.min(t, (0 - p0.y) / dy)
  return t === Infinity || t < 0 ? fallback : t
}

/**
 * Which end of the (world-space) line `a`-`b` has more free on-screen room
 * to push a label past — the tie-break-to-"right/up" rule from
 * `computeLineLabelLayout`'s doc comment, resolved in world space so callers
 * don't have to build the screen-space probe themselves. `0` = past `a`
 * (continuing away from `b`), `1` = past `b`.
 */
export function pickOutsideEnd(
  camera: THREE.Camera,
  a: THREE.Vector3,
  b: THREE.Vector3,
  viewportWidth: number,
  viewportHeight: number,
): 0 | 1 {
  const dir = b.clone().sub(a)
  if (dir.lengthSq() < 1e-18) return 1
  dir.normalize()
  const room0 = screenRoomBeyond(camera, a, dir.clone().multiplyScalar(-1), viewportWidth, viewportHeight)
  const room1 = screenRoomBeyond(camera, b, dir, viewportWidth, viewportHeight)
  return room1 >= room0 ? 1 : 0
}

// --------------------------------------------------------- §3: gesture plane
//
// HISTORY — the rule here has been corrected once, and the old rule's
// comments used to assert the opposite of what users actually want, so the
// reasoning is spelled out:
//
// The ORIGINAL rule ("view-facing", dimensions-playtest2.md §3's first cut,
// `viewFacingPlaneNormal`, since removed) built the drag-out plane as the
// plane through the baseline most face-on to the CAMERA — normal = the
// component of the view direction perpendicular to the baseline. That is a
// well-defined construction whose in-plane offset direction is exactly
// screen-parallel, and it is exactly wrong for a dimension: a dimension is
// DOCUMENT ink, and its plane must be anchored to the MODEL, not to
// whichever camera the user happened to be holding. From any oblique (e.g.
// ISO) camera the screen-parallel offset direction is a world-space
// diagonal — baseline along X viewed from the standard ISO gives offset
// direction (0, 1, 1)/sqrt(2), a 45-degree float matching nothing in the
// model (the angle-dimensions defect, reproduced verbatim in the
// maintainer's saved model). From a top/front/side view the screen-parallel
// plane happens to COINCIDE with an axis plane, which is why the defect was
// invisible to every axis-aligned-camera test.
//
// The CORRECTED rule: a linear dimension's working plane is always a
// MODEL-anchored plane through the baseline — the shared sketch plane when
// one was adopted, the arrow-key-locked plane when the user asked for one,
// and otherwise the best AXIS-ALIGNED candidate (`axisDimensionPlane`)
// chosen by where the cursor ray lands. SketchUp behaves the same way: its
// dimension strings live in the axis planes through the measured span, not
// in a screen-parallel free plane.

/** World axes, in candidate order (X, Y, Z — ties in `axisDimensionPlane`'s
 * face-on score keep the earlier candidate). */
const WORLD_AXES: readonly V3[] = [[1, 0, 0], [0, 1, 0], [0, 0, 1]]

/**
 * Once a working plane is latched for a gesture, a challenger candidate
 * must beat the incumbent's CURRENT score by this factor to take over —
 * the same acquire/release SHAPE as the snap system's `SNAP_RADIUS_PX` (8)
 * vs `SNAP_BREAK_RADIUS_PX` (16), which exists for exactly this class of
 * flicker: a stateless per-event re-decision oscillating across a soft
 * boundary.
 *
 * The VALUE is 1.25, not the snap system's 2, because the metric differs:
 * snap radii are screen distances, where 2x is a modest margin, while
 * these scores are alignment cosines bounded by each candidate's screen
 * foreshortening. Worked from the standard ISO view of an X baseline
 * (candidate offset directions project to screen magnitudes 0.85 and 0.74,
 * 56 degrees apart): with a 2x ratio a latched vertical plane could NEVER
 * be overthrown by a drag exactly along the flat plane's own on-screen
 * offset direction (challenger 0.74 vs 2 x 0.85 x cos(56) = 0.95) — the
 * rival plane becomes unreachable within the gesture, hysteresis decaying
 * into a dead-lock. 1.25 yields roughly a 20-degree angular dead-band
 * around the fresh-decision crossing for that same configuration: wide
 * enough that boundary jitter cannot flip the plane, narrow enough that a
 * deliberate re-aim always can.
 */
export const PLANE_SWITCH_RATIO = 1.25

/**
 * A latched plane may only be overthrown while the drag is DECISIVE: the
 * drag's perpendicular screen component is at least this fraction of the
 * whole drag from the first anchor. Right where the cursor crosses the
 * baseline's own screen line, the perpendicular component passes through
 * zero and its direction swings arbitrarily — an indecisive moment whose
 * score spikes no fixed switch ratio can absorb (see
 * `axisDimensionPlane`'s HYSTERESIS note) — and a drag running mostly
 * ALONG the baseline expresses no plane preference either way.
 */
export const DECISIVE_PERP_FRAC = 0.2

/**
 * The stronger bar a plane DEMOTED earlier in the same gesture (switched
 * away from) must clear to come back: challenger > this x incumbent's
 * current score, instead of `PLANE_SWITCH_RATIO`. Exists because a ratio
 * alone cannot make a monotonic drag no-returning: the score's drag
 * direction genuinely rotates THROUGH one candidate's screen direction and
 * onward toward another's on paths that skim past the baseline (measured
 * on the review's ISO repro sweep: after the mid-path excursion switched
 * planes, the abandoned plane's score recovered to ~1.4-1.5x the new
 * incumbent's by the sweep's end — a plain `PLANE_SWITCH_RATIO` would flip
 * straight back). 1.6 sits above that measured recovery (~1.5) yet below
 * the ~1.8 a genuinely reversed drag produces (re-aiming decisively along
 * the abandoned plane's own offset direction), so a deliberate change of
 * mind still returns, while a pass-through excursion cannot oscillate. A
 * hard per-gesture ban would guarantee no-return unconditionally but was
 * rejected: it would also ban the deliberate change of mind, and the only
 * way back would be cancelling the gesture.
 */
export const PLANE_RETURN_RATIO = 1.6

/**
 * Per-gesture working-plane memory for `axisDimensionPlane`'s hysteresis —
 * owned by the caller (one per drag gesture, reset when a new baseline
 * starts), mutated by the function as the drag evolves.
 */
export interface AxisPlaneDragState {
  /** The currently latched plane's normal, or null before first pick. */
  latched: V3 | null
  /** Normals of planes switched AWAY from earlier in this gesture — held
   * to `PLANE_RETURN_RATIO` instead of `PLANE_SWITCH_RATIO`. */
  demoted: V3[]
}

/** A fresh, empty drag state (start of a gesture). */
export function freshAxisPlaneDragState(): AxisPlaneDragState {
  return { latched: null, demoted: [] }
}

/**
 * The AXIS-ALIGNED working plane for a linear dimension's drag-out stage:
 * among the planes through the baseline `a` + unit `base` that contain (the
 * baseline-perpendicular component of) one world axis, pick the one the
 * cursor's drag direction means, and return it with the ray's pierce point
 * (the drag cursor, already ON the plane).
 *
 * Candidates: for each world axis `e` not parallel to `base`, the plane
 * through the baseline with normal `normalize(perp(e, base))` — the plane
 * containing the baseline that is closest to having `e` as its normal;
 * exactly the axis plane whenever `base ⊥ e` (the common case: dimensioning
 * an axis-aligned edge gives the flat plane and the vertical plane(s)
 * through it). A skew baseline still gets three well-defined candidates. A
 * candidate must be pierced by the ray in front of the camera (the pierce
 * point is what the offset tracks), or it is out entirely.
 *
 * SCORE — the user's own drag direction, on screen: `|v̂ · P(d)|`, where
 * `v` is the baseline-perpendicular component of the drag (from `a` to the
 * ray's pierce of the screen-parallel plane through `a`) projected into
 * the screen plane and normalized, `d = n × base` is the candidate's unit
 * in-plane offset direction, and `P(d) = d - (d·viewDir)viewDir` is its
 * screen projection, deliberately NOT normalized: a candidate whose offset
 * direction is foreshortened contributes proportionally less screen motion
 * per world unit, and one whose offset direction lies along the view axis
 * projects to nothing — it cannot be dragged and scores 0. This is the
 * gesture itself expressing intent WHERE the projection preserves it (see
 * the AMBIGUOUS-POSES limit below): from an oblique camera it is what
 * breaks the tie the camera creates in any camera-only measure (an ISO
 * view direction has equal-magnitude components on all three world axes,
 * so face-on-ness ties EXACTLY between candidates from ISO — a first
 * version of this function scored by `|n·viewDir|` and let array order
 * decide those ties, which is no rule at all). Before any real drag exists
 * (`v` degenerate: the cursor still on the baseline's own screen line, or
 * `a` behind the camera), the score falls back to `|P(d)|` alone — "most
 * draggable".
 *
 * AMBIGUOUS POSES — when the camera looks PERPENDICULAR to the baseline
 * (`viewDir · base = 0`: e.g. any pure-pitch orbit off a Front/Right/Top
 * view while dimensioning an edge running across the view), the drag
 * carries NO plane information, and no scoring function of the cursor ray
 * can recover any: every candidate offset direction lies in the 2D plane
 * perpendicular to the baseline, that plane then CONTAINS `viewDir`, so
 * every `P(d)` — and `v̂` itself, built by the same two projections —
 * collapses onto the single screen line perpendicular to the baseline's
 * image. Both candidate planes project onto the SAME screen strip: they
 * are visually indistinguishable in that pose, so the ambiguity is the
 * view's, not the score's. In this regime the score degenerates to
 * `|v̂ · P(d)| = |P(d)|`, and because `viewDir ∈ span(n, d)` makes
 * `(n·viewDir)² + (d·viewDir)² = 1`, that equals `|n·viewDir|` exactly:
 * the MOST FACE-ON plane wins, for any drag. That default is deliberate
 * and pinned by test: a dimension in the near-edge-on rival plane would be
 * barely readable from this camera, so the plane the user can actually
 * see is the right unprompted answer — and the arrow-key plane lock
 * (`lockedDimensionPlaneNormal`) is THE way to place into the other plane
 * from such a pose. The collapse is a neighbourhood, not a point: the
 * drag's discriminating power shrinks with `|viewDir·base|` (measured
 * still face-on-dominated at |dot| = 0.05, fully drag-decided at ISO's
 * 0.58), fading in smoothly with obliquity.
 *
 * HYSTERESIS — `state` is this gesture's plane memory
 * (`AxisPlaneDragState`; callers keep one per gesture and reset it when a
 * new baseline starts; omitting it evaluates statelessly). Three rules,
 * each measured against a real failure mode rather than assumed:
 *
 *  - LATCH + RATIO: the incumbent (latched) plane keeps winning until a
 *    challenger beats `PLANE_SWITCH_RATIO`x its current score. Without
 *    latching, any stateless per-move re-decision flickers across its
 *    decision boundary: the first version of this function
 *    (span-eligibility + camera-facing score) was confirmed flipping
 *    Y -> X -> Y and back across a monotonic one-way ISO drag, because the
 *    per-candidate eligibility regions are perspective-warped,
 *    non-complementary screen regions and its score was a constant of the
 *    drag.
 *  - DECISIVENESS: switching also requires the drag's perpendicular
 *    component to be at least `DECISIVE_PERP_FRAC` of the whole drag from
 *    `a` (`DECISIVE_PERP_FRAC`'s doc) — an indecisive drag (running along
 *    the baseline, or right at its screen line where the perpendicular
 *    direction swings) expresses no plane preference, so the incumbent
 *    simply stays.
 *  - DEMOTION: a plane switched away from earlier in this gesture must
 *    clear `PLANE_RETURN_RATIO` instead (see its doc comment for the
 *    measured why) — this is what makes a monotonic drag no-returning in
 *    practice while still allowing a deliberate change of mind back.
 *
 * An incumbent that stops being pierced by the ray drops out and the best
 * remaining candidate takes over.
 *
 * Returns `null` when no candidate plane is pierced at all (e.g. sighting
 * straight down the baseline, where every containing plane is edge-on) —
 * callers fall back to the raw snapped cursor, which keeps the gesture
 * total; the latch is left untouched so one degenerate event cannot reset
 * the drag's plane.
 */
export function axisDimensionPlane(
  a: V3,
  base: V3,
  rayOrigin: V3,
  rayDir: V3,
  viewDir: V3,
  state: AxisPlaneDragState | undefined = undefined,
): { normal: V3; hit: V3 } | null {
  // The screen-space drag direction: where the ray pierces the
  // screen-parallel plane through `a`, minus its along-baseline part,
  // projected into the screen plane. (This plane is a measuring probe for
  // the DIRECTION only — the working plane itself is never camera-anchored;
  // that was the original angle-dimensions defect.)
  let vHat: V3 | null = null
  let decisive = false
  const probe = rayPlaneIntersect(rayOrigin, rayDir, a, viewDir)
  if (probe !== null) {
    const drag = subV3(probe, a)
    const vScreen = perpComponentV3(perpComponentV3(drag, base), viewDir)
    vHat = normalizeV3(vScreen)
    const dragLen = Math.hypot(drag[0], drag[1], drag[2])
    const vLen = Math.hypot(vScreen[0], vScreen[1], vScreen[2])
    decisive = dragLen > 1e-9 && vLen >= DECISIVE_PERP_FRAC * dragLen
  }

  const candidates: { normal: V3; hit: V3; score: number }[] = []
  for (const axis of WORLD_AXES) {
    const n = normalizeV3(perpComponentV3(axis, base))
    if (n === null) continue // axis parallel to the baseline — no such plane
    const hit = rayPlaneIntersect(rayOrigin, rayDir, a, n)
    if (hit === null) continue // edge-on to the ray, or behind the camera
    const d = crossV3(n, base) // unit: n ⊥ base, both unit
    const dScreen = perpComponentV3(d, viewDir)
    const score =
      vHat !== null
        ? Math.abs(dotV3(vHat, dScreen))
        : Math.hypot(dScreen[0], dScreen[1], dScreen[2])
    candidates.push({ normal: n, hit, score })
  }
  if (candidates.length === 0) return null

  const samePlane = (m: V3, n: V3) => Math.abs(dotV3(m, n)) > 0.999
  // Each challenger is held to its own bar: PLANE_RETURN_RATIO if it was
  // demoted earlier in this gesture, PLANE_SWITCH_RATIO otherwise. The best
  // challenger is the one with the largest headroom over its bar.
  const barFor = (n: V3) =>
    state !== undefined && state.demoted.some((d) => samePlane(d, n)) ? PLANE_RETURN_RATIO : PLANE_SWITCH_RATIO
  let best = candidates[0]
  for (const c of candidates) {
    if (c.score / barFor(c.normal) > best.score / barFor(best.normal)) best = c
  }
  const incumbent =
    state?.latched != null
      ? candidates.find((c) => samePlane(c.normal, state.latched as V3))
      : undefined
  let picked = best
  if (incumbent !== undefined) {
    picked = incumbent
    if (
      decisive &&
      !samePlane(best.normal, incumbent.normal) &&
      best.score > barFor(best.normal) * incumbent.score
    ) {
      picked = best
      // The overthrown incumbent is demoted for the rest of the gesture.
      if (state !== undefined && !state.demoted.some((d) => samePlane(d, incumbent.normal))) {
        state.demoted.push(incumbent.normal)
      }
    }
  }
  if (state !== undefined) state.latched = picked.normal
  return { normal: picked.normal, hit: picked.hit }
}

/**
 * The working plane an arrow-key lock pins for a linear dimension: the
 * plane through the baseline closest to having world axis `axis` as its
 * normal — `normalize(perp(axis, base))`, exactly the locked axis plane
 * whenever the baseline is perpendicular to the axis. Same arrow → axis
 * convention as the draw tools (`arrowToAxis`: an arrow names the PLANE'S
 * OWN normal, so ArrowUp/blue is the flat plane). `null` when the baseline
 * is parallel to the axis — no plane with that normal can contain the
 * baseline, so the lock is unusable for this baseline and callers fall
 * through to the natural choice.
 */
export function lockedDimensionPlaneNormal(axis: 0 | 1 | 2, base: V3): V3 | null {
  return normalizeV3(perpComponentV3(WORLD_AXES[axis], base))
}

// -------------------------------------------------------- §4: radial geometry

/**
 * Tolerance (world units) within which a chord between two rim points is
 * treated as passing through the centre, i.e. a diameter
 * (docs/design/dimensions-playtest2.md §4's antipodal test): the greater of
 * an absolute floor (so a vanishingly small circle doesn't make every chord
 * "antipodal") and 2% of the circle's own radius.
 */
export function antipodalTolerance(radius: number): number {
  return Math.max(1e-6, 0.02 * radius)
}

/** Distance from `p` to the infinite line through `a`/`b`, or the distance
 * to `a` when `a`/`b` coincide (no line to measure against). */
export function distPointToLine(p: V3, a: V3, b: V3): number {
  const ab = subV3(b, a)
  const abLenSq = dotV3(ab, ab)
  if (abLenSq < 1e-18) {
    const d = subV3(p, a)
    return Math.hypot(d[0], d[1], d[2])
  }
  const t = dotV3(subV3(p, a), ab) / abLenSq
  const closest: V3 = [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t]
  const d = subV3(p, closest)
  return Math.hypot(d[0], d[1], d[2])
}

/** True iff the chord `rimA`-`rimB` of a circle centred at `center` with
 * `radius` passes close enough to the centre to be treated as a diameter
 * (docs/design/dimensions-playtest2.md §4's antipodal test/tolerance). */
export function chordPassesNearCentre(center: V3, radius: number, rimA: V3, rimB: V3): boolean {
  return distPointToLine(center, rimA, rimB) <= antipodalTolerance(radius)
}

export interface RadialSegments {
  /** The "measured" run: `[centre, anchor]` for a radius (centre to rim),
   * `[antipode, anchor]` for a diameter (rim to rim, straight through the
   * centre) — docs/design/dimensions-playtest2.md §4's "the drawn geometry
   * must show the measurement". */
  measured: [V3, V3]
  /** The far end of `measured` — the true centre for a radius, so callers
   * can draw a small centre tick there; the antipodal rim point for a
   * diameter. */
  farEnd: V3
}

/**
 * The measured-run geometry for a radial dimension (docs/design/
 * dimensions-playtest2.md §4) — shared by `DimensionTool`'s live preview and
 * `SceneRenderer`'s committed render so the two can never draw a different
 * shape for the same stored `(centre, anchor, kind)`. The LEADER segment
 * (from `anchor` outward to the label) is not this function's concern —
 * callers add it the same way a linear dimension's own line is added.
 */
export function buildRadialGeometry(center: V3, anchor: V3, kind: 'radius' | 'diameter'): RadialSegments {
  if (kind === 'diameter') {
    const antipode: V3 = [
      2 * center[0] - anchor[0],
      2 * center[1] - anchor[1],
      2 * center[2] - anchor[2],
    ]
    return { measured: [antipode, anchor], farEnd: antipode }
  }
  return { measured: [center, anchor], farEnd: center }
}

/** Half-extent (world units) of a radial dimension's centre tick mark. */
export const CENTER_TICK_HALF = 0.03

/** Appends a small "+" cross-tick centred at `center`, lying in the plane
 * with unit normal `planeNormal` — the "small centre cross/tick" a Radius
 * dimension draws at the true centre (docs/design/dimensions-playtest2.md
 * §4). No-op (nothing pushed) if `planeNormal` is degenerate. */
export function pushCenterTick(positions: number[], center: V3, planeNormal: V3, half: number): void {
  const arbitrary: V3 = Math.abs(planeNormal[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0]
  const u = normalizeV3(perpComponentV3(arbitrary, planeNormal))
  if (u === null) return
  const v = crossV3(planeNormal, u)
  positions.push(
    center[0] - u[0] * half, center[1] - u[1] * half, center[2] - u[2] * half,
    center[0] + u[0] * half, center[1] + u[1] * half, center[2] + u[2] * half,
  )
  positions.push(
    center[0] - v[0] * half, center[1] - v[1] * half, center[2] - v[2] * half,
    center[0] + v[0] * half, center[1] + v[1] * half, center[2] + v[2] * half,
  )
}
