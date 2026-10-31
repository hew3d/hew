/**
 * DimensionTool — SketchUp-style linear + radial dimensions
 * (docs/design/dimensions-text.md "Tools & UX", with the gesture-plane and
 * radial-disambiguation fixes from docs/design/dimensions-playtest2.md §3/§4
 * layered on top — see those sections for the "why"). Kernel entity:
 * `Annotation::LinearDimension` / `Annotation::RadialDimension`
 * (crates/kernel/src/annotation.rs); rendered by
 * `SceneRenderer.refreshAnnotations()`.
 *
 * Gesture (linear — SketchUp's real dimension gesture, three presses):
 *   1. First click: picks anchor A (any inference snap point). Live rubber-
 *      band from A to the cursor previews the baseline.
 *   2. Second click: picks anchor B. A hovered sketch's plane is adopted at
 *      the FIRST click (sketches on any plane — mirrors `TapeMeasureTool`'s
 *      hover-adopted non-ground sketch plane), unless an arrow-key plane
 *      lock is active (a lock beats adoption, the draw tools' own rule).
 *   3. Pointer move drags the dimension line's perpendicular OFFSET out of
 *      the A-B line (SketchUp's "drag out" gesture). The plane the offset
 *      drags in — the dimension's WORKING plane — is always MODEL-anchored
 *      (the angle-dimensions fix; annotationLayout.ts §3 documents why the
 *      earlier screen-parallel "view-facing" rule was itself the defect),
 *      in priority order:
 *        a. the arrow-key-locked plane through the baseline
 *           (`lockedDimensionPlaneNormal`; same arrow → plane-normal-axis
 *           convention as the draw tools — ArrowRight/Left = the red/green
 *           upright planes, ArrowUp = the blue flat plane, same arrow
 *           again or ArrowDown unlocks);
 *        b. the sketch plane adopted at the first click;
 *        c. the best AXIS-ALIGNED plane through the baseline for where the
 *           cursor ray lands (`axisDimensionPlane`) — never the ground
 *           plane, and never a camera-anchored free plane. From a pose
 *           looking PERPENDICULAR to the baseline the candidate planes
 *           project onto the same screen strip and the drag cannot say
 *           which is meant (annotationLayout.ts §3's AMBIGUOUS-POSES
 *           note): the most face-on plane wins deliberately, and the
 *           arrow lock is the way to choose the other one.
 *      Live preview shows extension lines, the dimension line, arrowheads,
 *      and the live `formatLength` readout.
 *   4. Third click commits: `add_linear_dimension(a, b, offset, plane)`.
 *      Degenerate (cursor still on the A-B line, zero offset) refuses with a
 *      status hint rather than committing an unplaceable dimension.
 *
 * Gesture (linear, from a line — SketchUp's edge gesture): a first click ON a
 * straight line (an `on-edge` snap, not one of its points) takes the line's
 * two ends as A and B at once (`snapEdgeEndpoints`) and goes straight to
 * step 3; a press-drag-release on the line places the dimension in one
 * motion. A drawn circle/arc's rim keeps the radial gesture below.
 *
 * Gesture (radial — click a drawn circle/arc, dimensions-playtest2.md §4):
 *   1. First click on a drawn (unextruded) sketch circle/arc's rim OR its
 *      analytic center snap starts a PENDING `'have-curve'` stage — it does
 *      NOT commit to a radial dimension yet (the old bug: entering "radial
 *      mode" on the first click meant the second click could only ever
 *      supply a leader direction, never a real second measurement point).
 *   2. Pointer move live-previews whatever the SECOND click would currently
 *      produce (`_classify`) — a plain leader (radius/diameter, drawn
 *      center-to-rim or rim-to-rim per `buildRadialGeometry`) while
 *      hovering free space, or an ordinary linear-dimension preview while
 *      hovering another point on the same rim.
 *   3. Tab toggles the DEFAULT radius/diameter preference while pending
 *      (only takes effect if the eventual second click lands off the curve
 *      entirely — an explicit center/antipodal click always wins with its
 *      own definite kind).
 *   4. Second click decides, per the disambiguation table:
 *      off the curve (free space, or the bare interior of a face)
 *                                  -> radial dimension, default kind Radius
 *        for an arc / Diameter for a full circle (SketchUp's rule),
 *        overridable by Tab.
 *      the curve's centre           -> Radius, anchored at the FIRST click's
 *        rim point (or, reversed — centre first, rim second — the SECOND
 *        click's rim point).
 *      an antipodal rim point       -> Diameter (the chord passes within
 *        `chordPassesNearCentre`'s tolerance of the centre).
 *      any other rim point          -> an ordinary LINEAR dimension of that
 *        chord (transitions to `'have-b'`, same drag-offset-then-click as
 *        any other linear dimension).
 *      anything else identifiable   -> an ordinary linear dimension to it.
 *
 * A drawn sketch curve is not a document tree node (`NodeId` is
 * Object/Group/Instance only — sketches aren't), so a radial dimension's
 * anchor is always a FREE anchor (`node: None`): it is captured exact at
 * creation but will not geometrically re-anchor if the source sketch is
 * later edited. This is an inherent limit of D1's `Anchor` model (matching
 * the design doc's own scope — sketches were never part of the anchor node
 * space), not something this tool works around.
 *
 * Esc cancels the current stage; a fresh click always starts over.
 */
import { t } from '@lingui/core/macro'
import { shownAnnotationIds } from '../viewport/shownAnnotations'
import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  planeFromSketch,
  axisDrawPlane,
  isGroundPlane,
  isPointOnDrawPlane,
  groundDrawPlane,
  SketchPickCache,
  type DrawPlane,
} from './drawPlane'
import { formatLength } from '../settings/units'
import { crossV3, normalizeV3, facePlaneBasis, rayPlaneIntersect, type V3 } from '../viewport/geoHelpers'
import { friendlyErrorText } from '../kernelErrors'
import {
  axisDimensionPlane,
  freshAxisPlaneDragState,
  lockedDimensionPlaneNormal,
  buildRadialGeometry,
  chordPassesNearCentre,
  findAlignmentSnap,
  type DimensionLineCandidate,
} from '../viewport/annotationLayout'
import { nextIdlePlaneLock, axisLockColorName, radiusReadout, diameterReadout } from './moveInput'
import { worldFaceNormal } from './faceDraw'
import { ctxGround, ctxOfFace, ctxOfSketch, planeOfCtx, readPlaneUnits, type PlaneUnitsScene } from './planeUnits'
import { projectPointOntoPlane } from './tapeOffset'
import { snapEdgeEndpoints } from './snapEdge'
import { axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'

/** Rubber-band preview color — matches `fatLine.ts`'s `PREVIEW_LINE_STYLE`
 * blue used by the draw tools' own gesture previews. */
export const PREVIEW_COLOR = 0x4d90ff

/** Snap kinds eligible to be honored OFF the `have-a` gesture plane, once
 *  one is frozen (design v1.1 Lane E "Dimensions on-plane") — deliberate,
 *  precise point inferences the user aimed at directly. Every other kind
 *  (`on-edge`, `on-face`, `on-axis`, `on-guide`, and the `plane`/`ground`
 *  synthetic fallbacks) is projected onto the frozen plane before use — see
 *  `_haveAPoint`. Eligibility is necessary but not sufficient: a member of
 *  this set is honored only when its own displacement off the plane is
 *  actually VISIBLE from the current camera (`_offPlaneDisplacementVisible`)
 *  — a maintainer playtest finding (a Top-view framed-wall corner has
 *  several endpoints stacked straight down the view ray — slab, sole plate,
 *  stud top, top plates — and the old rule honored whichever one the snap
 *  happened to resolve, landing the dimension off-plane with no visible cue
 *  that it had). */
const HAVE_A_OFF_PLANE_KINDS = new Set(['endpoint', 'midpoint', 'center', 'intersection', 'quadrant'])

/** Minimum angle (degrees) between a precise off-plane point's own
 *  displacement (its raw position minus its projection onto the frozen
 *  `have-a` plane) and the camera's view direction for that displacement to
 *  count as VISIBLE (`_offPlaneDisplacementVisible`) — below this the
 *  displacement reads as coplanar on screen no matter how large it is in
 *  world space (the stacked-corner defect: a stud top 7cm under the top
 *  plates, viewed from directly above, has its ENTIRE displacement running
 *  along the view ray). 2 degrees, not 0: floating-point/camera noise can
 *  put a displacement a hair off exactly parallel even when the user's
 *  intent (and every practical framing) is a dead-straight Top/Front/Right
 *  view, and an honored point that then renders one pixel off-plane would
 *  be worse than a projected one that renders exactly on it. */
const OFF_PLANE_VISIBLE_MIN_DEG = 2
const OFF_PLANE_VISIBLE_MIN_SIN = Math.sin((OFF_PLANE_VISIBLE_MIN_DEG * Math.PI) / 180)

/** Screen-pixel radius (`ALIGN_SNAP_PX`) within which the have-b drag's
 *  current offset point must pass a candidate dimension's own line for the
 *  row-alignment snap to take it (`_alignmentTolerance`/`findAlignmentSnap`
 *  in `annotationLayout.ts`) — the same acquire-radius SHAPE as ordinary
 *  point snapping, just tool-local rather than routed through SnapService
 *  (this is a dimension-to-dimension relationship SnapService's kernel-fed
 *  candidates know nothing about). */
const ALIGN_SNAP_PX = 12

/** World-space alignment-snap tolerance used when no live pixel scale has
 *  been fed in yet (`updateDiskScale` never called — every legacy unit
 *  test, and the very first frame after this tool activates) — a fixed
 *  fallback rather than skipping the snap outright, so it is still testable
 *  and usable before the first `updateDiskScale` tick lands. */
const ALIGN_FALLBACK_TOLERANCE_M = 0.15

export type OnAnnotationCreated = () => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Minimum world-space offset magnitude a linear dimension's placement must
 * clear before it can be committed — below this the a1/b1 dimension line is
 * indistinguishable from the a-b baseline and no plane can be derived from
 * it (see `planeFromLineAndOffset`). */
const MIN_OFFSET = 1e-4

/** A click within this fraction of a curve's own radius (floored, so a tiny
 * circle isn't unreasonably fussy) counts as landing on the CENTRE rather
 * than the rim — analytic center snaps report the exact center coordinate,
 * so in practice this only has to separate "basically zero" from "basically
 * radius". */
function centerClickTolerance(radius: number): number {
  return Math.max(1e-4, radius * 0.25)
}

/** Default leader length (world units) for a radial dimension committed
 * without an explicit outward drag (a direct click on the curve's centre or
 * an antipodal rim point, dimensions-playtest2.md §4) — a fraction of the
 * curve's own radius, floored so a tiny circle still gets a readable leader. */
const DEFAULT_LEADER_LEN_FRAC = 0.4
const DEFAULT_LEADER_LEN_MIN = 0.1

function anchorNodeFromSnap(snap: Snap): { kind: number; id: bigint } | null {
  if (snap.instance !== undefined) return { kind: 2, id: snap.instance }
  if (snap.object !== undefined) return { kind: 0, id: snap.object }
  return null
}

function sub(a: V3, b: V3): V3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
function add(a: V3, b: V3): V3 {
  return [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
function scale(v: V3, s: number): V3 {
  return [v[0] * s, v[1] * s, v[2] * s]
}
function dot(a: V3, b: V3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
function length(v: V3): number {
  return Math.hypot(v[0], v[1], v[2])
}
/** `v`'s component perpendicular to unit `d`. */
function perpComponent(v: V3, d: V3): V3 {
  return sub(v, scale(d, dot(v, d)))
}

/**
 * A plane containing the A-B baseline and the (already-perpendicularized)
 * `offset` direction — the plane a linear dimension's line/extension lines
 * are drawn in. `null` if A/B coincide or `offset` is too small to derive a
 * direction from (see `MIN_OFFSET`).
 */
function planeFromLineAndOffset(a: V3, b: V3, offset: V3): { point: V3; normal: V3 } | null {
  const dir = normalizeV3(sub(b, a))
  if (dir === null) return null
  const perp = perpComponent(offset, dir)
  if (length(perp) < MIN_OFFSET) return null
  const normal = normalizeV3(crossV3(dir, perp))
  if (normal === null) return null
  return { point: a, normal }
}

/** A drawn (unextruded) sketch curve resolved from a snap: its analytic
 * circle/arc plus the owning sketch's plane. */
interface ResolvedCurve {
  center: V3
  radius: number
  planePoint: V3
  planeNormal: V3
  /** Owning sketch, so the disambiguation logic can query the curve's own
   * chain (arc-vs-full-circle, `isFullCircle`). `null` for a circle read
   * from a plane's listing (a drawing on a solid's face). */
  sketch: bigint | null
  /** The curve chain's own handle (`sketch_curve_geom`'s `curve` id) — lets
   * `isFullCircle` ask the kernel for every one of the chain's edges
   * directly (`sketch_curve_edges`), regardless of which specific point the
   * resolving snap named (a quadrant, the centre, or a plain rim point all
   * resolve the SAME curve handle). `null` for a listing circle. */
  curveHandle: bigint | null
  /** The plane context the circle was listed under (a listing circle). */
  ctx: string | null
  /** Whether a listing circle is a whole circle rather than an arc or a run
   *  of one. */
  fullCircle: boolean
}

/** How near (m) a snap must lie to a listed circle's rim or center to name it. */
const LISTED_CURVE_EPS = 1e-5

/** Distance from `p` to the segment `a`-`b`. */
function distToSegment(p: V3, a: V3, b: V3): number {
  const ab = sub(b, a)
  const l2 = dot(ab, ab)
  const t = l2 > 0 ? Math.max(0, Math.min(1, dot(sub(p, a), ab) / l2)) : 0
  return length(sub(p, add(a, scale(ab, t))))
}

/**
 * Resolve `snap` to a circle in a plane's `plane_units_json` listing — the
 * unified-drafting reading of a drawn curve on ANY plane (a face's drawing
 * included, where no free sketch holds it). The planes tried: `ctxHint` alone
 * when given (the first click's own plane), else the plane of the sketch the
 * snap names, the face under `ray`, and the ground. The circle is the run
 * whose rim or center the snap point lies on.
 */
function tryResolveListedCurve(
  wasmScene: WasmScene,
  snap: Snap,
  ray: Ray | undefined,
  ctxHint: string | null,
): ResolvedCurve | null {
  const ctxs: string[] = []
  try {
    if (ctxHint !== null) {
      ctxs.push(ctxHint)
    } else {
      if (snap.sketch !== undefined) {
        const c = ctxOfSketch(wasmScene, snap.sketch, null)
        if (c !== null) ctxs.push(c)
      }
      if (ray !== undefined && typeof wasmScene.pick_face === 'function') {
        const pick = wasmScene.pick_face(
          ray.origin[0], ray.origin[1], ray.origin[2],
          ray.direction[0], ray.direction[1], ray.direction[2],
        )
        if (pick !== undefined) {
          try {
            const c = pick.instance() === undefined ? ctxOfFace(wasmScene, pick.object(), pick.face(), null) : null
            if (c !== null) ctxs.push(c)
          } finally {
            pick.free()
          }
        }
      }
      ctxs.push(ctxGround())
    }
  } catch {
    return null
  }
  if (typeof wasmScene.plane_units_json !== 'function') return null
  const P: V3 = [snap.x, snap.y, snap.z]
  for (const ctx of ctxs) {
    const pl = planeOfCtx(ctx)
    const pu = readPlaneUnits(wasmScene as unknown as PlaneUnitsScene, ctx)
    if (pl === null || pu === null) continue
    if (Math.abs(dot(sub(P, pl.point), pl.normal)) > LISTED_CURVE_EPS) continue
    let best: { err: number; unit: number; run: number } | null = null
    // A snap ON a drawing edge (a rim click between facet vertices, a facet
    // midpoint) lies on the CHORD, off the true circle by the sagitta: name
    // the circle by the snapped edge's run, whatever the point's distance to
    // the circle. Vertex/quadrant/center snaps keep the on-circle check.
    if (snap.kind === 'on-edge' || snap.kind === 'midpoint') {
      pu.units.forEach((u, ui) => {
        u.curves.forEach((run, ri) => {
          // A null radius is an ellipse (a circle in a stretched instance):
          // nothing radial to measure.
          if (run.kind !== 'circle' || run.radius === null) return
          for (const k of run.edges) {
            const e = pu.edgeByKey.get(k)?.edge
            if (e === undefined) continue
            const err = distToSegment(P, e.a, e.b)
            if (err <= LISTED_CURVE_EPS && (best === null || err < best.err)) best = { err, unit: ui, run: ri }
          }
        })
      })
    }
    if (best === null) {
      pu.units.forEach((u, ui) => {
        u.curves.forEach((run, ri) => {
          if (run.kind !== 'circle' || run.radius === null) return
          const d = length(sub(P, run.center))
          const err = Math.min(d, Math.abs(d - run.radius))
          if (err <= LISTED_CURVE_EPS && (best === null || err < best.err)) best = { err, unit: ui, run: ri }
        })
      })
    }
    if (best === null) continue
    const { unit, run: ri } = best as { err: number; unit: number; run: number }
    const run = pu.units[unit].curves[ri]
    if (run.radius === null) continue
    // Whole circle: the run's facets close on themselves.
    const ends = run.edges.map((k) => pu.edgeByKey.get(k)?.edge)
    const closed = ends.length > 0 && ends.every((e, i) => e !== undefined && ends.some((f, j) => j !== i && f !== undefined && length(sub(e.b, f.a)) < 1e-6))
    return {
      center: run.center,
      radius: run.radius,
      planePoint: pl.point,
      planeNormal: pl.normal,
      sketch: null,
      curveHandle: null,
      ctx,
      fullCircle: closed,
    }
  }
  return null
}

/**
 * Resolve `snap` to the drawn circle/arc it names, or `null` if it names no
 * curve (a plain line/vertex/face snap, or a polygon chain with no captured
 * analytic geometry — `sketch_curve_geom` is circles/arcs only).
 */
function tryResolveCurve(
  wasmScene: WasmScene,
  snap: Snap,
  ray?: Ray,
  ctxHint: string | null = null,
): ResolvedCurve | null {
  return tryResolveSketchCurve(wasmScene, snap) ?? tryResolveListedCurve(wasmScene, snap, ray, ctxHint)
}

/** The free-sketch reading: the snap names a sketch curve by handle, whose
 *  exact geometry the kernel answers. */
function tryResolveSketchCurve(wasmScene: WasmScene, snap: Snap): ResolvedCurve | null {
  if (snap.sketch === undefined) return null
  let curveHandle: bigint | undefined
  if (snap.elementKind === 'sketch-curve' && snap.sketchCurve !== undefined) {
    curveHandle = snap.sketchCurve
  } else if (snap.elementKind === 'sketch-edge' && snap.element !== undefined) {
    curveHandle = wasmScene.sketch_edge_curve(snap.sketch, snap.element)
  }
  if (curveHandle === undefined) return null
  const geom = wasmScene.sketch_curve_geom(snap.sketch, curveHandle)
  if (geom === undefined) return null
  const plane = wasmScene.sketch_plane(snap.sketch)
  if (plane === undefined) return null
  return {
    center: [geom[0], geom[1], geom[2]],
    radius: geom[3],
    planePoint: [plane[0], plane[1], plane[2]],
    planeNormal: [plane[3], plane[4], plane[5]],
    sketch: snap.sketch,
    curveHandle,
    ctx: null,
    fullCircle: false,
  }
}

/** Whether `a`/`b` describe the same analytic circle: center + radius +
 * PLANE all close enough to be the same click target. Two concentric
 * circles of the same radius on two DIFFERENT planes (e.g. perpendicular
 * sketches sharing an origin) are an ordinary modelling situation, not a
 * degenerate one — center+radius alone doesn't distinguish them, and
 * treating a rim point from one as belonging to the other silently
 * corrupts the classification (e.g. `nearestRimPoint` re-projects the
 * SECOND curve's own rim point onto the FIRST curve's plane using the
 * wrong center/radius/plane triple, which can degenerate to an arbitrary
 * fallback direction when the point projects exactly onto the axis).
 * Plane comparison is by NORMAL DIRECTION ONLY (parallel or antiparallel —
 * a sketch's winding/orientation isn't part of a curve's identity): once
 * the centers already match (checked first) and a curve's center always
 * lies in its own sketch's plane, two normals parallel to within `1e-6` of
 * `|dot|=1` through that same shared point describe the identical plane. */
function sameCurve(a: ResolvedCurve, b: ResolvedCurve): boolean {
  if (length(sub(a.center, b.center)) >= 1e-6) return false
  if (Math.abs(a.radius - b.radius) >= 1e-6) return false
  const na = normalizeV3(a.planeNormal)
  const nb = normalizeV3(b.planeNormal)
  if (na === null || nb === null) return false
  return Math.abs(Math.abs(dot(na, nb)) - 1) < 1e-6
}

/** The point on `curve`'s rim closest to (the in-plane projection of)
 * `point` — SketchUp's radial-dimension leader touches the circumference,
 * not the center, however precisely the user actually clicked. Falls back
 * to an arbitrary in-plane direction when `point` projects exactly onto the
 * center (degenerate — no direction to project along). */
function nearestRimPoint(curve: ResolvedCurve, point: V3): V3 {
  const rel = sub(point, curve.center)
  const relOnPlane = perpComponent(rel, curve.planeNormal)
  let dir = normalizeV3(relOnPlane)
  if (dir === null) {
    const basis = facePlaneBasis(curve.planeNormal)
    dir = basis?.u ?? [1, 0, 0]
  }
  return add(curve.center, scale(dir, curve.radius))
}

/**
 * Best-effort "is this curve chain a full, closed circle (vs. an open arc)"
 * check, built entirely from EXISTING wasm-api surface (`sketch_curve_edges`
 * + `sketch_edge_endpoints`) — no new wasm-api surface, per this repo's
 * ground rules. `sketch_curve_edges` takes the curve handle DIRECTLY
 * (unlike `sketch_curve_chain`, which needs a sample edge — not always
 * available: a click on a circle's exact quadrant/centre snap resolves the
 * curve with no edge of its own), so this works regardless of which
 * specific point on the curve the resolving snap named. A chain is closed
 * when every edge's end point matches some OTHER edge's start point (within
 * a tight tolerance): an open arc always has (at least) one edge whose end
 * matches nothing else in the chain.
 *
 * Conservatively returns `false` (arc, the SketchUp-matching default for
 * anything uncertain) when the wasm scene doesn't expose these calls (a
 * test double that only mocks the calls its own test needs) or any call
 * fails — dimensions-playtest2.md §4 only asks for this as the DEFAULT kind
 * when the user drags straight off the curve into free space; every other
 * row of the disambiguation table has its own definite kind that doesn't
 * depend on this at all.
 */
function isFullCircle(wasmScene: WasmScene, sketch: bigint, curveHandle: bigint): boolean {
  if (typeof wasmScene.sketch_curve_edges !== 'function' || typeof wasmScene.sketch_edge_endpoints !== 'function') {
    return false
  }
  try {
    const chain = wasmScene.sketch_curve_edges(sketch, curveHandle)
    if (chain.length === 0) return false
    const starts: V3[] = []
    const ends: V3[] = []
    for (const edge of chain) {
      const ep = wasmScene.sketch_edge_endpoints(sketch, edge)
      if (ep === undefined) return false
      starts.push([ep[0], ep[1], ep[2]])
      ends.push([ep[3], ep[4], ep[5]])
    }
    const EPS = 1e-6
    for (let i = 0; i < chain.length; i++) {
      const matched = starts.some((s, j) => j !== i && length(sub(ends[i], s)) < EPS)
      if (!matched) return false
    }
    return true
  } catch {
    return false
  }
}

type LinearStage =
  | { kind: 'idle' }
  | {
      kind: 'have-a'
      aNode: { kind: number; id: bigint } | null
      aPoint: V3
      gesturePlane: DrawPlane | null
      /** Whether `gesturePlane` came from a hovered SKETCH's own plane
       *  (`_resolveGesturePlane`), as opposed to the design v1.1 freeze's
       *  lock/on-face/view-aligned fallback (`_freezeGesturePlane`). Only an
       *  adopted plane carries forward as `have-b`'s hard override
       *  (`_workingPlane`'s existing behavior, unchanged); a freeze-derived
       *  plane instead just SEEDS `have-b`'s `axisDimensionPlane` latch so
       *  the drag starts continuous with it, without pinning the drag to a
       *  plane computed before any baseline existed. */
      gesturePlaneAdopted: boolean
    }
  | {
      kind: 'have-b'
      aNode: { kind: number; id: bigint } | null
      aPoint: V3
      bNode: { kind: number; id: bigint } | null
      bPoint: V3
      offset: V3
      gesturePlane: DrawPlane | null
    }
  | {
      kind: 'have-curve'
      curve: ResolvedCurve
      /** The first click's point: a rim point (`nearestRimPoint`), or the
       * exact center when `firstIsCenter`. */
      firstPoint: V3
      firstIsCenter: boolean
      /** Radius/diameter preference set by Tab — `null` until Tab is
       * pressed, meaning "use the arc/full-circle auto default"; only
       * consulted for the off-curve-into-space row of the disambiguation
       * table (every other row has its own definite kind). */
      preferredKind: 'radius' | 'diameter' | null
      /** Last hover point/snap, kept so `_updatePreview` can re-run the same
       * classification the next click would use, without needing its own
       * copy of the pointer state. */
      lastCursor: V3
      lastSnap: Snap
    }

/** Second-click disambiguation result (dimensions-playtest2.md §4's table). */
type Classification =
  | { kind: 'radius' | 'diameter'; anchor: V3; leaderDir: V3 }
  | { kind: 'linear'; aPoint: V3; bPoint: V3 }
  | { kind: 'ignore' }

export class DimensionTool implements Tool {
  readonly name = 'Dimension'
  /** Click-drag-release works as well as click-click (Tool.dragCommits). */
  readonly dragCommits = true

  /** The current editing context: resolves a clicked line's endpoints inside
   *  a component instance (`snapEdgeEndpoints`). */
  private _editContext: EditContext = { kind: 'top' }
  setEditContext(ctx: EditContext): void {
    this._editContext = ctx
  }

  statusHint(): string {
    const lockedAxis = this.planeLock !== null ? axisLockColorName(this.planeLock) : null
    const lockNote = lockedAxis !== null ? t`Locked to the ${lockedAxis} plane — same arrow or ArrowDown unlocks.` : ''
    const withLock = (sentence: string): string => (lockNote === '' ? sentence : `${sentence} ${lockNote}`)
    switch (this.stage.kind) {
      case 'have-a':
        return withLock(t`Click the second point.`)
      case 'have-b':
        return withLock(t`Drag out the dimension line, then click to place it — arrows lock the plane.`)
      case 'have-curve': {
        const kind = this.stage.preferredKind ?? this._defaultRadialKind(this.stage)
        return kind === 'radius'
          ? t`Drag out the leader, then click to place — Tab for diameter.`
          : t`Drag out the leader, then click to place — Tab for radius.`
      }
      default:
        return withLock(t`Click a point to dimension from, a line to dimension it, or a drawn circle/arc for a radius dimension.`)
    }
  }

  private stage: LinearStage = { kind: 'idle' }
  private preview: THREE.Group
  private previewLine: THREE.LineSegments | null = null
  private wasmScene: WasmScene
  private onCreated: OnAnnotationCreated
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement
  private readonly _sketchPickCache = new SketchPickCache()
  /** The current camera's view (forward) direction, refreshed once per
   * render frame via `updateCamera` (mirrors `ScaleTool.updateGripScale`'s
   * live-camera-feed pattern) — `null` until the first frame after this
   * tool becomes active, so a caller that never wires a camera in (every
   * existing unit test) gets the exact pre-fix behavior rather than a
   * guessed view direction (dimensions-playtest2.md §3). */
  private _viewDir: V3 | null = null
  /** Arrow-key plane lock for the linear gesture's working plane — the SAME
   * user choice as the draw tools' idle plane lock (`nextIdlePlaneLock`'s
   * toggle semantics, `arrowToAxis`'s key → axis mapping: an arrow names
   * the PLANE'S OWN normal axis). Survives gesture cancel/commit like the
   * draw tools' lock; cleared by the same arrow, ArrowDown, or an idle
   * Escape. */
  private planeLock: 0 | 1 | 2 | null = null
  /** The CURRENT have-b drag's plane memory (`axisDimensionPlane`'s
   * hysteresis state: the latched plane + planes demoted by switching away
   * — see its doc comment) — reset whenever a new baseline starts and on
   * cancel, so latching never leaks across gestures. */
  private _axisDragState = freshAxisPlaneDragState()
  /** Whether the snap most recently handed to `onPointerMove`/`onPointerDown`
   *  was projected onto the frozen `have-a` gesture plane rather than used
   *  as-is (`Tool.snapProjected`, TapeMeasureTool's own disclosure pattern)
   *  — reset every event, set only by `_haveAPoint`. */
  private _snapProjected = false

  /** The snap last seen on hover — `Viewport.publishSnapCues`'s established
   *  `'lastSnap' in activeTool` opt-in (PaintTool/OffsetTool/PushPullTool's
   *  own pattern): reading the cue's kind from HERE instead of the raw
   *  resolved snap lets this tool report a kind of its own (`'aligned'`,
   *  the dimension-row alignment snap) without SnapService needing to know
   *  anything about dimensions. Mirrors the raw snap exactly except while
   *  the alignment snap is active, so every other cue stays byte-identical
   *  to before this field existed. */
  lastSnap: Snap | null = null

  /** Live world-per-pixel feed (`Tool.updateDiskScale`'s doc — feature-
   *  detected by the Viewport render loop, called once per frame before this
   *  tool draws anything): the alignment snap's `ALIGN_SNAP_PX` acquire
   *  radius is a SCREEN distance, and `onPointerMove` only ever carries a
   *  per-pixel cursor ray, never the camera itself or its projection. `null`
   *  until the first post-activation frame (every legacy unit test, which
   *  never wires a camera in) — `_alignmentTolerance` falls back to
   *  `ALIGN_FALLBACK_TOLERANCE_M` in that case. */
  private _worldPerPixelFn: ((dist: number) => number) | null = null
  private _cameraPos: V3 | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCreated: OnAnnotationCreated,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCreated = onCreated
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  /** Live camera feed, called once per render frame while this tool is
   * active (feature-detected by `Viewport.tsx`, mirroring
   * `updateDiskScale`) — `axisDimensionPlane`'s face-on tie-break needs the
   * camera's actual forward direction, which this tool has no other way to
   * observe (`onPointerMove`/`onPointerDown` only carry a per-pixel cursor
   * ray, not the camera itself). */
  updateCamera(camera: THREE.Camera): void {
    const dir = new THREE.Vector3()
    camera.getWorldDirection(dir)
    this._viewDir = [dir.x, dir.y, dir.z]
  }

  /** Live pixel-scale feed for the dimension-row alignment snap — see
   *  `_worldPerPixelFn`'s doc. Not used to size any visible disk (unlike
   *  every other implementor of this hook); this tool just needs the same
   *  live `worldPerPixel(dist)` callback the on-screen-constant tools do. */
  updateDiskScale(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    this._worldPerPixelFn = worldPerPixel
    this._cameraPos = [camera.position.x, camera.position.y, camera.position.z]
  }

  capturingInput(): boolean {
    return this.stage.kind !== 'idle'
  }

  /**
   * Enter `have-b` on the baseline A→B: the offset drag starts from here.
   * Design v1.1 "Dimensions on-plane": only a genuinely ADOPTED sketch plane
   * carries forward as `have-b`'s hard override (`_workingPlane`'s
   * pre-existing behavior, unchanged). A lock/on-face/view-aligned freeze
   * instead just SEEDS the fresh drag state's latch — so the offset drag
   * starts continuous with the plane B was just constrained to — but ONLY
   * when B actually WAS constrained by it (`seedFromFrozen`): an
   * off-plane-honored precise point (`HAVE_A_OFF_PLANE_KINDS`, e.g. a real
   * endpoint) never touched the frozen plane at all, so seeding from it would
   * bias `have-b` toward a plane the second click had no real relationship
   * to — purely incidental if the (camera-only, no-drag-information) freeze
   * axis happens to be perpendicular to the resulting baseline. No lock check
   * needed either: while locked, `_workingPlane` reads `this.planeLock`
   * directly, regardless of `gesturePlane`/the latch.
   */
  private _beginOffsetDrag(
    aNode: { kind: number; id: bigint } | null,
    aPoint: V3,
    bNode: { kind: number; id: bigint } | null,
    bPoint: V3,
    frozenPlane: DrawPlane | null,
    adopted: boolean,
    seedFromFrozen: boolean,
  ): void {
    this._axisDragState = freshAxisPlaneDragState() // a NEW baseline starts its drag unlatched
    if (!adopted && frozenPlane !== null && seedFromFrozen) {
      const baseDir = normalizeV3(sub(bPoint, aPoint))
      if (baseDir !== null && Math.abs(dot(baseDir, frozenPlane.normal)) < 1e-6) {
        this._axisDragState.latched = frozenPlane.normal
      }
    }
    this.stage = {
      kind: 'have-b',
      aNode,
      aPoint,
      bNode,
      bPoint,
      offset: [0, 0, 0],
      gesturePlane: adopted ? frozenPlane : null,
    }
    this._updatePreview()
  }

  /** See `_snapProjected`'s doc — `Tool.snapProjected`'s cue-label
   *  disclosure. */
  snapProjected(): boolean {
    return this._snapProjected
  }

  snapConstraint(ray?: Ray): { constraintPlane?: { point: V3; normal: V3 }; offPlanePoints?: boolean } | null {
    if (this.stage.kind === 'have-a') {
      // `_haveAPlane` is the LIVE lock (if pressed BETWEEN the two clicks,
      // it wins immediately) over whatever `_freezeGesturePlane` computed
      // at the first click — design v1.1 "Dimensions on-plane": the second
      // point's snap search is constrained to it, with `offPlanePoints` so
      // a precise point (endpoint/midpoint/center/quadrant/intersection)
      // elsewhere in space is still reachable — `_haveAPoint` then projects
      // everything else onto this same plane before it's used, so the
      // committed baseline always lies in one plane no matter which
      // candidate won the snap.
      const plane = this._haveAPlane(this.stage)
      if (plane !== null) {
        return { constraintPlane: { point: plane.origin, normal: plane.normal }, offPlanePoints: true }
      }
      return null
    }
    if (this.stage.kind === 'have-b') {
      // The drag's WORKING plane (lock > adopted sketch plane > axis-aligned
      // default — `_workingPlane`) constrains the cursor's snapping too, so
      // the drag can never fall through to the ground plane.
      const plane = this._workingPlane(this.stage, ray)
      if (plane !== null) return { constraintPlane: { point: plane.point, normal: plane.normal } }
      return null
    }
    if (this.stage.kind === 'have-curve') return null
    if (ray === undefined) return null
    const handle = this._sketchPickCache.pickFor(this.wasmScene, ray)
    if (handle !== null) {
      const plane = planeFromSketch(this.wasmScene, handle)
      if (plane !== null && !plane.ground) {
        return { constraintPlane: { point: plane.origin, normal: plane.normal } }
      }
    }
    return null
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (snap === null) return
    // Reset every event — only the `have-a` branch below (via `_haveAPoint`)
    // ever sets this true, so a stale `true` from a prior have-a hover can
    // never leak into another stage's inference-chip disclosure.
    this._snapProjected = false
    const cursor: V3 = [snap.x, snap.y, snap.z]
    // Mirrors `snap` exactly except when the have-b branch below overrides
    // it to report the alignment snap's own `'aligned'` cue kind — see the
    // field's own doc.
    this.lastSnap = snap

    if (this.stage.kind === 'have-b') {
      const dir = sub(this.stage.bPoint, this.stage.aPoint)
      const dirLen = length(dir)
      if (dirLen > 1e-9) {
        const unit: V3 = [dir[0] / dirLen, dir[1] / dirLen, dir[2] / dirLen]
        let cursorForOffset = cursor
        // Compute the offset against the WORKING plane (lock > adopted
        // sketch plane > axis-aligned default — `_workingPlane`): for the
        // lock/axis cases the RAW cursor ray is intersected with it live
        // rather than trusting whatever the snap system resolved; the
        // adopted-plane case keeps the snap-constrained cursor exactly as
        // before (`snapConstraint` already pins it to that plane, so
        // `_workingPlane` reports no pierce point of its own).
        const plane = this._workingPlane(this.stage, ray)
        if (plane !== null && plane.hit !== null) cursorForOffset = plane.hit
        this.stage.offset = perpComponent(sub(cursorForOffset, this.stage.aPoint), unit)

        // Dimension-row alignment snap (SketchUp parity): if the drag's
        // current offset point sits near an existing dimension's own line
        // that is parallel to this baseline and shares this working plane,
        // snap collinear with it — see `findAlignmentSnap`. Needs a real
        // working plane to compare normals against; skipped entirely
        // without one (the same rare ray-parallel-to-baseline degenerate
        // case `_workingPlane` itself falls back for).
        if (plane !== null) {
          const currentA1 = add(this.stage.aPoint, this.stage.offset)
          const tol = this._alignmentTolerance(currentA1)
          const aligned = findAlignmentSnap(
            this.stage.aPoint, unit, currentA1, plane.normal, this._alignmentCandidates(), tol,
          )
          if (aligned !== null) {
            this.stage.offset = aligned
            this.lastSnap = { ...snap, kind: 'aligned' }
          }
        }
      }
      this._updatePreview()
      const dist = length(sub(this.stage.bPoint, this.stage.aPoint))
      this.onMeasurementCb(formatLength(dist))
      return
    }

    if (this.stage.kind === 'have-curve') {
      this.stage.lastCursor = cursor
      this.stage.lastSnap = snap
      this._updatePreview()
      const cls = this._classify(this.stage, snap, cursor)
      if (cls.kind === 'radius' || cls.kind === 'diameter') {
        const value = cls.kind === 'diameter' ? this.stage.curve.radius * 2 : this.stage.curve.radius
        this.onMeasurementCb(
          cls.kind === 'diameter' ? diameterReadout(formatLength(value)) : radiusReadout(formatLength(value)),
        )
      } else if (cls.kind === 'linear') {
        const dist = length(sub(cls.bPoint, cls.aPoint))
        this.onMeasurementCb(formatLength(dist))
      } else {
        this.onMeasurementCb('')
      }
      return
    }

    if (this.stage.kind === 'have-a') {
      // Precise off-plane point kinds are honored as-is; every other kind
      // is projected onto the frozen gesture plane (design v1.1 "Dimensions
      // on-plane" — see `_haveAPoint`), so the live rubber-band preview
      // always matches what a click would actually commit.
      this._updatePreview(this._haveAPoint(snap, this._haveAPlane(this.stage)))
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (snap === null) return
    this._snapProjected = false
    const point: V3 = [snap.x, snap.y, snap.z]

    if (this.stage.kind === 'idle') {
      const curve = tryResolveCurve(this.wasmScene, snap, ray)
      if (curve !== null) {
        const firstIsCenter = length(sub(point, curve.center)) < centerClickTolerance(curve.radius)
        const firstPoint = firstIsCenter ? curve.center : nearestRimPoint(curve, point)
        this.stage = {
          kind: 'have-curve',
          curve,
          firstPoint,
          firstIsCenter,
          preferredKind: null,
          lastCursor: firstPoint,
          lastSnap: snap,
        }
        this._updatePreview()
        return
      }
      // A click on a line itself (not a point on it) dimensions the whole
      // line, SketchUp's edge gesture: its two ends are A and B at once, and
      // the gesture goes straight to the offset drag. The plane is frozen
      // exactly as for a first point.
      const edge = snap.kind === 'on-edge' ? snapEdgeEndpoints(this.wasmScene, snap, this._editContext) : null
      if (edge !== null && length(sub(edge[1], edge[0])) >= 1e-9) {
        const node = anchorNodeFromSnap(snap)
        const frozen = this._freezeGesturePlane(ray, snap, edge[0])
        this._beginOffsetDrag(node, edge[0], node, edge[1], frozen?.plane ?? null, frozen?.adopted ?? false, true)
        return
      }
      // The first click always freezes a gesture plane now (design v1.1
      // "Dimensions on-plane" — see `_freezeGesturePlane`), not just when a
      // sketch happens to be hovered: without one, the second point's snap
      // search was unconstrained and could land on the ground plane behind
      // whatever the user was actually aiming at (e.g. a wall top in Top
      // view) — the Dimensions bug the design fixes.
      const frozen = this._freezeGesturePlane(ray, snap, point)
      this.stage = {
        kind: 'have-a',
        aNode: anchorNodeFromSnap(snap),
        aPoint: point,
        gesturePlane: frozen?.plane ?? null,
        gesturePlaneAdopted: frozen?.adopted ?? false,
      }
      this._updatePreview()
      return
    }

    if (this.stage.kind === 'have-a') {
      // Same projection the live preview already showed (`_haveAPoint`) —
      // the committed baseline can never land off the frozen gesture plane
      // just because the raw snap did.
      const bPoint = this._haveAPoint(snap, this._haveAPlane(this.stage))
      if (length(sub(bPoint, this.stage.aPoint)) < 1e-9) return // same point twice — ignore
      this._beginOffsetDrag(
        this.stage.aNode, this.stage.aPoint, anchorNodeFromSnap(snap), bPoint,
        this.stage.gesturePlane, this.stage.gesturePlaneAdopted, !HAVE_A_OFF_PLANE_KINDS.has(snap.kind),
      )
      return
    }

    if (this.stage.kind === 'have-b') {
      this._commitLinear(this.stage)
      return
    }

    if (this.stage.kind === 'have-curve') {
      const cls = this._classify(this.stage, snap, point)
      if (cls.kind === 'ignore') return
      if (cls.kind === 'linear') {
        // Same "lock overrides adoption" rule as the idle first click.
        const gesturePlane = this.planeLock !== null ? null : this._resolveGesturePlane(ray)
        this._axisDragState = freshAxisPlaneDragState() // a NEW baseline starts its drag unlatched
        this.stage = {
          kind: 'have-b',
          aNode: null,
          aPoint: cls.aPoint,
          bNode: anchorNodeFromSnap(snap),
          bPoint: cls.bPoint,
          offset: [0, 0, 0],
          gesturePlane,
        }
        this._updatePreview()
        return
      }
      this._commitRadial(this.stage.curve, cls.anchor, cls.leaderDir, cls.kind)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      // Draw-tool parity: an idle Escape clears the lock FIRST; cancelling
      // an in-progress gesture keeps the lock (it is a durable aiming
      // choice, not gesture state).
      if (this.stage.kind === 'idle' && this.planeLock !== null) {
        this.planeLock = null
        return
      }
      this.cancel()
      return
    }
    if (
      (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') &&
      this.stage.kind !== 'have-curve'
    ) {
      // Arrow-key plane lock, sharing the draw tools' exact convention
      // (`nextIdlePlaneLock`): the arrow names the PLANE'S OWN normal axis
      // (ArrowRight/Left = red/green upright planes, ArrowUp = blue flat
      // plane), the same arrow again or ArrowDown clears. Radial gestures
      // aren't planar drags, so the arrows are left alone in 'have-curve'.
      this.planeLock = nextIdlePlaneLock(this.planeLock, ev.key)
      this._updatePreview()
      return
    }
    if (ev.key === 'Tab' && this.stage.kind === 'have-curve') {
      ev.preventDefault()
      const current = this.stage.preferredKind ?? this._defaultRadialKind(this.stage)
      this.stage.preferredKind = current === 'radius' ? 'diameter' : 'radius'
      this._updatePreview()
      const value = this.stage.preferredKind === 'diameter' ? this.stage.curve.radius * 2 : this.stage.curve.radius
      this.onMeasurementCb(
        this.stage.preferredKind === 'diameter' ? diameterReadout(formatLength(value)) : radiusReadout(formatLength(value)),
      )
    }
  }

  cancel(): void {
    this.stage = { kind: 'idle' }
    this._axisDragState = freshAxisPlaneDragState()
    this._clearPreview()
    this.onMeasurementCb('')
  }

  private _resolveGesturePlane(ray: Ray): DrawPlane | null {
    const handle = this._sketchPickCache.pickFor(this.wasmScene, ray)
    if (handle !== null) {
      const plane = planeFromSketch(this.wasmScene, handle)
      if (plane !== null && !plane.ground) return plane
    }
    return null
  }

  /**
   * The gesture plane frozen at the FIRST click of a linear dimension
   * (design v1.1 Lane E "Dimensions on-plane"). Without this, `have-a`'s
   * `snapConstraint` offered no plane at all unless a sketch happened to be
   * hovered, so the SECOND point's snap search — and its ground-plane
   * fallback — was unconstrained: a Top-view dimension across a wall's top
   * edge could resolve its second endpoint on the literal ground instead of
   * at the wall's own height, the "Dimensions bug" this fixes.
   *
   * Priority, matching the have-b drag's own `_workingPlane` precedence:
   *   0. An adopted sketch plane under the cursor (`_resolveGesturePlane`)
   *      — skipped entirely while a lock is active, same "lock beats
   *      adoption" rule the draw tools use.
   *   1. An active arrow-key plane lock: the literal axis plane through
   *      `aPoint` (`axisDrawPlane`, the draw tools' own idle-lock
   *      convention). This is only a SNAP-SEARCH constraint for the second
   *      click — once a real baseline exists, `_workingPlane` refines the
   *      committed working plane into the baseline-perpendicular
   *      `lockedDimensionPlaneNormal` instead, which this method has no
   *      baseline yet to compute.
   *   2. The first click's own snap landing `on-face` on a real object
   *      face (never a sketch-region fill, which reports `elementKind ===
   *      'sketch-region'` instead of `'face'`): that face's plane, through
   *      `aPoint`.
   *   3. The axis plane through `aPoint` whose normal is most face-on to
   *      the current camera (Top → the horizontal plane at the click's own
   *      height; Front → the XZ plane; ISO → whichever axis wins) — `null`
   *      only when no camera has been wired in yet (every legacy unit test
   *      that never calls `updateCamera`), mirroring `_workingPlane`'s own
   *      "no camera → no computed plane" fallback.
   *
   * `adopted` is true only for case 0 (a real hovered sketch) — the ONLY
   * case whose plane should carry forward as `have-b`'s hard override; the
   * lock/on-face/view-aligned cases instead only SEED `have-b`'s
   * `axisDimensionPlane` latch (see the `have-a` stage's own doc).
   */
  private _freezeGesturePlane(
    ray: Ray,
    snap: Snap,
    aPoint: V3,
  ): { plane: DrawPlane; adopted: boolean } | null {
    if (this.planeLock !== null) return { plane: axisDrawPlane(this.planeLock, aPoint), adopted: false }

    const adopted = this._resolveGesturePlane(ray)
    if (adopted !== null) return { plane: adopted, adopted: true }

    if (snap.elementKind === 'face' && snap.object !== undefined && snap.element !== undefined) {
      const normal = worldFaceNormal(this.wasmScene, snap.object, snap.element, snap.instance ?? null)
      if (normal !== null) {
        const plane = this._planeThroughPoint(aPoint, normal)
        if (plane !== null) return { plane, adopted: false }
      }
    }

    const axis = this._viewAlignedAxis()
    return axis !== null ? { plane: axisDrawPlane(axis, aPoint), adopted: false } : null
  }

  /** A `DrawPlane` through `origin` with unit `normal` — the general
   *  point+normal constructor `axisDrawPlane` doesn't cover (its normal is
   *  always a literal drawing axis). Mirrors `planeFromSketch`'s own tail:
   *  the exact `groundDrawPlane()` when the result IS the ground plane, else
   *  `null` for a degenerate normal. */
  private _planeThroughPoint(origin: V3, normal: V3): DrawPlane | null {
    if (isGroundPlane(origin, normal)) return groundDrawPlane()
    const basis = facePlaneBasis(normal)
    if (basis === null) return null
    return { origin, normal, u: basis.u, v: basis.v, ground: false }
  }

  /** The world axis (0=X, 1=Y, 2=Z) most nearly PARALLEL to the current
   *  camera view direction — i.e. the axis the camera looks most straight
   *  along, so the plane NORMAL to it is the most face-on candidate (Top
   *  view looks down Z → the horizontal XY-normal plane; Front view looks
   *  along Y → the XZ plane). `null` when no camera has been wired in
   *  (`updateCamera` never called — every legacy unit test). */
  private _viewAlignedAxis(): 0 | 1 | 2 | null {
    if (this._viewDir === null) return null
    const [ax, ay, az] = [Math.abs(this._viewDir[0]), Math.abs(this._viewDir[1]), Math.abs(this._viewDir[2])]
    if (ax >= ay && ax >= az) return 0
    if (ay >= az) return 1
    return 2
  }

  /**
   * The plane `have-a` uses RIGHT NOW, for both `snapConstraint`'s advertised
   * constraint and `_haveAPoint`'s projection (design v1.1 "Dimensions
   * on-plane"): a LIVE arrow-key lock always wins, even one pressed AFTER
   * the first click ("between clicks", per the design) — never mind
   * whatever `_freezeGesturePlane` computed at click time, exactly the same
   * "lock beats adoption" precedence the draw tools use everywhere else.
   * Absent a live lock, the plane frozen at the first click
   * (`stage.gesturePlane`) stands.
   */
  private _haveAPlane(stage: Extract<LinearStage, { kind: 'have-a' }>): DrawPlane | null {
    if (this.planeLock !== null) return axisDrawPlane(this.planeLock, stage.aPoint)
    return stage.gesturePlane
  }

  /**
   * Resolve the actual point a `have-a`-stage snap contributes (design v1.1
   * "Dimensions on-plane", as corrected by the maintainer's stacked-corner
   * playtest finding): a precise point kind (`HAVE_A_OFF_PLANE_KINDS`) is
   * honored exactly as reported, even off `plane`, but ONLY when its own
   * displacement off `plane` is actually VISIBLE from the current camera
   * (`_offPlaneDisplacementVisible`) — mirrors `TapeMeasureTool`'s
   * `_measureTarget`/`projectPointOntoPlane` pairing, gated by KIND *and*
   * visibility rather than by plane-membership or kind alone. Every other
   * kind, and every invisible-displacement precise point, is projected onto
   * `plane`.
   *
   * `plane === null` (no gesture plane was frozen — no camera wired in yet,
   * the legacy pre-Lane-E test path) returns the raw point unchanged, so
   * every existing test that never calls `updateCamera` keeps its exact
   * prior behavior.
   *
   * Sets `_snapProjected` to whether this call actually moved the point —
   * the single write site for it, so `onPointerMove`/`onPointerDown`'s
   * per-event reset (`this._snapProjected = false`) is the only other
   * place that touches the field.
   */
  private _haveAPoint(snap: Snap, plane: DrawPlane | null): V3 {
    const raw: V3 = [snap.x, snap.y, snap.z]
    if (plane === null || isPointOnDrawPlane(raw, plane)) {
      this._snapProjected = false
      return raw
    }
    if (HAVE_A_OFF_PLANE_KINDS.has(snap.kind) && this._offPlaneDisplacementVisible(raw, plane)) {
      this._snapProjected = false
      return raw
    }
    this._snapProjected = true
    return projectPointOntoPlane(raw, plane.origin, plane.normal)
  }

  /**
   * Whether a precise point's displacement off `plane` (`raw` minus its own
   * projection onto `plane`) would be VISIBLE from the current camera — the
   * maintainer's stacked-corner playtest finding: in a Top view (parallel
   * projection, looking straight down), every endpoint in a framed wall's
   * corner — slab, sole plate, stud top, top plates — projects to the SAME
   * screen pixel, so honoring whichever one the snap happens to resolve
   * (off the frozen horizontal plane) commits a dimension that LOOKS
   * coplanar on screen but isn't. Visible iff the angle between the
   * displacement and `this._viewDir` is at least `OFF_PLANE_VISIBLE_MIN_DEG`
   * — computed as `|cross(disp̂, vieŵ)| >= sin(that angle)` rather than a
   * dot-product threshold, since it's the SINE of the angle-from-parallel
   * that vanishes at both parallel AND antiparallel (a displacement running
   * either straight toward or straight away from the camera along the view
   * ray is equally invisible — that's exactly the depth axis).
   *
   * `this._viewDir === null` (no camera wired in yet) can't be scored at
   * all — conservatively treated as NOT visible, so an off-plane precise
   * point never gets honored on the strength of a camera this tool hasn't
   * actually observed. `raw` already on `plane` is handled by the caller
   * before this is ever reached, so a degenerate (near-zero) displacement
   * here is unreachable in practice; if it somehow occurs, there is nothing
   * to hide, so it counts as visible.
   */
  private _offPlaneDisplacementVisible(raw: V3, plane: DrawPlane): boolean {
    if (this._viewDir === null) return false
    const projected = projectPointOntoPlane(raw, plane.origin, plane.normal)
    const dispDir = normalizeV3(sub(raw, projected))
    if (dispDir === null) return true
    const viewDir = normalizeV3(this._viewDir)
    if (viewDir === null) return false
    return length(crossV3(dispDir, viewDir)) >= OFF_PLANE_VISIBLE_MIN_SIN
  }

  /**
   * The linear gesture's WORKING plane for the current baseline — the plane
   * `snapConstraint` advertises and the have-b drag computes its offset in.
   * Priority: arrow-key lock > adopted sketch plane > best axis-aligned
   * plane for the cursor ray (the angle-dimensions fix; the class doc's
   * step 3 and annotationLayout.ts §3 carry the full rationale).
   *
   * `hit` is the cursor ray's pierce point on the returned plane when this
   * call computed one itself (the lock/axis cases, given a ray) so the
   * caller can reuse it without a second intersection; `null` in the
   * adopted-plane case, where the snap system already constrains the cursor
   * to the plane and the snapped cursor should be used as-is.
   *
   * Returns `null` (no working plane — the offset falls back to the raw
   * snapped cursor) when the baseline is degenerate, or when neither a
   * usable lock nor an adopted plane exists and either no camera has been
   * registered (callers that never wire one in — every legacy unit test —
   * keep the exact pre-camera behavior) or the ray pierces no candidate
   * plane at all (e.g. sighting straight down the baseline).
   */
  private _workingPlane(
    stage: Extract<LinearStage, { kind: 'have-b' }>,
    ray: Ray | undefined,
  ): { point: V3; normal: V3; hit: V3 | null } | null {
    const base = normalizeV3(sub(stage.bPoint, stage.aPoint))
    if (base === null) return null
    if (this.planeLock !== null) {
      const normal = lockedDimensionPlaneNormal(this.planeLock, base)
      if (normal !== null) {
        const hit = ray !== undefined ? rayPlaneIntersect(ray.origin, ray.direction, stage.aPoint, normal) : null
        return { point: stage.aPoint, normal, hit }
      }
      // The locked axis is parallel to the baseline — no plane with that
      // normal can contain it; fall through to the natural choice.
    }
    if (stage.gesturePlane !== null) {
      return { point: stage.gesturePlane.origin, normal: stage.gesturePlane.normal, hit: null }
    }
    if (this._viewDir === null || ray === undefined) return null
    const picked = axisDimensionPlane(
      stage.aPoint, base, ray.origin, ray.direction, this._viewDir, this._axisDragState,
    )
    // A momentarily degenerate ray keeps the latch (the state is untouched
    // on a null return): the drag's plane must not silently reset because
    // one event's ray grazed every candidate.
    if (picked === null) return null
    return { point: stage.aPoint, normal: picked.normal, hit: picked.hit }
  }

  /** World-space alignment-snap tolerance at `atPoint` (`ALIGN_SNAP_PX`
   *  converted via the live `updateDiskScale` feed, or `ALIGN_FALLBACK_
   *  TOLERANCE_M` before the first tick of it has landed — see `_worldPer
   *  PixelFn`'s doc). */
  private _alignmentTolerance(atPoint: V3): number {
    if (this._worldPerPixelFn === null || this._cameraPos === null) return ALIGN_FALLBACK_TOLERANCE_M
    const dist = length(sub(atPoint, this._cameraPos))
    return this._worldPerPixelFn(dist) * ALIGN_SNAP_PX
  }

  /** Every EXISTING linear dimension in the document, as the
   *  `DimensionLineCandidate` shape `findAlignmentSnap` needs: its own drawn
   *  line (anchor points already offset out — the same `a1`/`b1` convention
   *  `getLinearDimensionEndpoints` reports) plus its working-plane normal.
   *  Built fresh every have-b pointer move (no caching) — the document can
   *  change between moves (undo, another dimension committed) and this list
   *  is never large enough for that to matter. A candidate missing any
   *  accessor (a stale/hidden id, or a test double that doesn't mock one) is
   *  skipped rather than throwing. */
  private _alignmentCandidates(): DimensionLineCandidate[] {
    const scene = this.wasmScene
    if (typeof scene.annotation_ids !== 'function') return []
    const out: DimensionLineCandidate[] = []
    for (const id of shownAnnotationIds(scene)) {
      if (scene.annotation_kind(id) !== 'linear') continue
      const a = scene.annotation_anchor_point(id, 0)
      const b = scene.annotation_anchor_point(id, 1)
      const offset = scene.annotation_offset(id)
      const plane = scene.annotation_plane(id)
      if (a === undefined || b === undefined || offset === undefined || plane === undefined) continue
      const off: V3 = [offset[0], offset[1], offset[2]]
      out.push({
        a1: add([a[0], a[1], a[2]], off),
        b1: add([b[0], b[1], b[2]], off),
        planeNormal: [plane[3], plane[4], plane[5]],
      })
    }
    return out
  }

  /** The kind the off-curve-into-space row of the disambiguation table would
   * default to right now (dimensions-playtest2.md §4: Radius for an arc,
   * Diameter for a full circle), ignoring any Tab override — used both to
   * seed Tab's first press and to describe the current default in
   * `statusHint`. */
  private _defaultRadialKind(stage: Extract<LinearStage, { kind: 'have-curve' }>): 'radius' | 'diameter' {
    const { curve } = stage
    const full = curve.sketch !== null && curve.curveHandle !== null
      ? isFullCircle(this.wasmScene, curve.sketch, curve.curveHandle)
      : curve.fullCircle
    return full ? 'diameter' : 'radius'
  }

  /** Default leader direction for a radial dimension committed without an
   * explicit outward drag (dimensions-playtest2.md §4: a direct click on
   * the centre, or on an antipodal rim point) — continues straight out past
   * `anchor`, away from the centre, by a fixed fraction of the radius. */
  private _defaultLeaderDir(curve: ResolvedCurve, anchor: V3): V3 {
    const dir = normalizeV3(sub(anchor, curve.center)) ?? [1, 0, 0]
    const len = Math.max(curve.radius * DEFAULT_LEADER_LEN_FRAC, DEFAULT_LEADER_LEN_MIN)
    return scale(dir, len)
  }

  /**
   * The second-click disambiguation table (dimensions-playtest2.md §4),
   * evaluated against `point`/`snap` for the pending `stage`. Shared by
   * `onPointerMove` (live preview — "what would clicking HERE do") and
   * `onPointerDown` (the actual commit decision), so the preview can never
   * show a different shape than what committing would actually produce.
   */
  private _classify(stage: Extract<LinearStage, { kind: 'have-curve' }>, snap: Snap, point: V3): Classification {
    const { curve, firstPoint, firstIsCenter } = stage
    const curve2 = tryResolveCurve(this.wasmScene, snap, undefined, curve.ctx)
    const same = curve2 !== null && sameCurve(curve, curve2)
    if (same) {
      const isCenter2 = length(sub(point, curve.center)) < centerClickTolerance(curve.radius)
      if (firstIsCenter) {
        // Reverse order: centre first, then a rim point -> Radius.
        if (isCenter2) return { kind: 'ignore' } // centre clicked twice
        const anchor = nearestRimPoint(curve, point)
        return { kind: 'radius', anchor, leaderDir: this._defaultLeaderDir(curve, anchor) }
      }
      if (isCenter2) {
        return { kind: 'radius', anchor: firstPoint, leaderDir: this._defaultLeaderDir(curve, firstPoint) }
      }
      const rim2 = nearestRimPoint(curve, point)
      if (length(sub(rim2, firstPoint)) < 1e-9) return { kind: 'ignore' } // same rim point twice
      if (chordPassesNearCentre(curve.center, curve.radius, firstPoint, rim2)) {
        return { kind: 'diameter', anchor: firstPoint, leaderDir: this._defaultLeaderDir(curve, firstPoint) }
      }
      return { kind: 'linear', aPoint: firstPoint, bPoint: rim2 }
    }
    if (firstIsCenter) {
      // No rim anchor to build a radial dimension from — the only sane
      // reading of "centre, then some unrelated point" is a plain distance.
      return { kind: 'linear', aPoint: firstPoint, bPoint: point }
    }
    // A click on the bare interior of the face the curve is drawn on names
    // nothing to measure to either: drawn on a face, the curve is read exactly
    // as on the ground. A face anywhere else (a side wall, another solid off
    // the curve's plane) is a point to measure to.
    const onCurvePlane =
      Math.abs(dot(sub(point, curve.center), curve.planeNormal)) <= 1e-6 * Math.max(1, curve.radius)
    const unidentified =
      (snap.object === undefined && snap.instance === undefined && snap.sketch === undefined) ||
      (snap.kind === 'on-face' &&
        onCurvePlane &&
        snap.elementKind !== 'sketch-curve' &&
        snap.elementKind !== 'sketch-edge')
    if (unidentified) {
      // Off the curve, into free space — SketchUp's one-entity radial flow.
      const leaderDir = perpComponent(sub(point, firstPoint), curve.planeNormal)
      const kind = stage.preferredKind ?? this._defaultRadialKind(stage)
      return { kind, anchor: firstPoint, leaderDir }
    }
    return { kind: 'linear', aPoint: firstPoint, bPoint: point }
  }

  private _commitLinear(stage: Extract<LinearStage, { kind: 'have-b' }>): void {
    const plane = planeFromLineAndOffset(stage.aPoint, stage.bPoint, stage.offset)
    if (plane === null) {
      this.onToast(t`Drag away from the line to set the dimension's offset`)
      return
    }
    try {
      this.wasmScene.add_linear_dimension(
        stage.aNode?.kind ?? -1,
        stage.aNode?.id ?? 0n,
        new Float64Array(stage.aPoint),
        stage.bNode?.kind ?? -1,
        stage.bNode?.id ?? 0n,
        new Float64Array(stage.bPoint),
        new Float64Array(stage.offset),
        new Float64Array([plane.point[0], plane.point[1], plane.point[2], plane.normal[0], plane.normal[1], plane.normal[2]]),
        undefined,
      )
      this.onCreated()
    } catch (err) {
      const reason = friendlyErrorText(err)
      this.onToast(t`Couldn't create dimension: ${reason}`)
    }
    this.cancel()
  }

  private _commitRadial(curve: ResolvedCurve, anchor: V3, leaderDir: V3, kind: 'radius' | 'diameter'): void {
    try {
      this.wasmScene.add_radial_dimension(
        -1,
        0n,
        new Float64Array(anchor),
        kind,
        new Float64Array(curve.center),
        curve.radius,
        new Float64Array([
          curve.planePoint[0], curve.planePoint[1], curve.planePoint[2],
          curve.planeNormal[0], curve.planeNormal[1], curve.planeNormal[2],
        ]),
        new Float64Array(leaderDir),
        undefined,
      )
      this.onCreated()
    } catch (err) {
      const reason = friendlyErrorText(err)
      this.onToast(t`Couldn't create radial dimension: ${reason}`)
    }
    this.cancel()
  }

  /**
   * The rubber-band preview's own color right now: the active plane lock's
   * axis color (`AXIS_LOCK_COLOR_NAMES`'s same red/green/blue, sourced from
   * `axisColorsForTheme` — the exact theme-aware color the draw tools'
   * own locked previews use, NOT `MoveTool`'s hardcoded non-theme-aware
   * `AXIS_COLOR`) while a lock is actually driving the working plane, else
   * the neutral `PREVIEW_COLOR`. Deliberately keyed on `this.planeLock`
   * alone (a LIVE lock), not on `gesturePlane`/`gesturePlaneAdopted` — a
   * plane adopted from a hovered sketch, chosen by the camera's own
   * view-aligned fallback, or seeded from an on-face freeze stays neutral;
   * only the user's own explicit arrow-key choice gets colored, per the
   * maintainer's playtest note. Only `have-a`/`have-b` use this (their
   * baseline/dimension-and-extension-line previews are exactly what a plane
   * lock affects); the radial gesture's preview stays neutral unconditionally
   * (arrows are left alone in `have-curve` — see `onKey`).
   */
  private _previewColor(): number {
    if (this.planeLock === null) return PREVIEW_COLOR
    return axisColorsForTheme(getResolvedTheme())[this.planeLock]
  }

  /** Rebuild the live rubber-band preview from the current stage. */
  private _updatePreview(hoverPoint?: V3): void {
    this._clearPreview()
    let positions: number[] | null = null
    let color: number = PREVIEW_COLOR

    if (this.stage.kind === 'have-a' && hoverPoint !== undefined) {
      positions = [...this.stage.aPoint, ...hoverPoint]
      color = this._previewColor()
    } else if (this.stage.kind === 'have-b') {
      const { aPoint, bPoint, offset } = this.stage
      const a1 = add(aPoint, offset)
      const b1 = add(bPoint, offset)
      positions = [...aPoint, ...a1, ...bPoint, ...b1, ...a1, ...b1]
      color = this._previewColor()
    } else if (this.stage.kind === 'have-curve') {
      const cls = this._classify(this.stage, this.stage.lastSnap, this.stage.lastCursor)
      if (cls.kind === 'radius' || cls.kind === 'diameter') {
        // Preview the SAME geometry a commit would draw (dimensions-
        // playtest2.md §4: "both the live preview and the committed render
        // change") — the measured run (centre-to-rim / rim-to-rim) plus the
        // leader continuing out to the label.
        const geo = buildRadialGeometry(this.stage.curve.center, cls.anchor, cls.kind)
        const end = add(cls.anchor, cls.leaderDir)
        positions = [...geo.measured[0], ...geo.measured[1], ...cls.anchor, ...end]
      } else if (cls.kind === 'linear') {
        positions = [...cls.aPoint, ...cls.bPoint]
      }
    }

    if (positions === null || positions.length === 0) return
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3))
    // A rubber band is transient gesture feedback, not document ink — kept
    // `depthTest: false` deliberately (dimensions-playtest2.md §1: every
    // other tool's in-progress preview draws on top too; this is NOT the
    // overlay-depth bug findings 2/3 fix, and must not be "fixed" to match
    // the now-depth-tested committed annotations).
    const mat = new THREE.LineBasicMaterial({ color, depthTest: false })
    this.previewLine = new THREE.LineSegments(geo, mat)
    this.preview.add(this.previewLine)
  }

  private _clearPreview(): void {
    if (this.previewLine === null) return
    this.previewLine.geometry.dispose()
    if (this.previewLine.material instanceof THREE.Material) this.previewLine.material.dispose()
    this.preview.remove(this.previewLine)
    this.previewLine = null
  }
}
