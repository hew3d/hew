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
 *      off the curve (free space)  -> radial dimension, default kind Radius
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
import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { planeFromSketch, SketchPickCache, type DrawPlane } from './drawPlane'
import { formatLength } from '../settings/units'
import { crossV3, normalizeV3, facePlaneBasis, rayPlaneIntersect, type V3 } from '../viewport/geoHelpers'
import { friendlyErrorText } from '../kernelErrors'
import { axisDimensionPlane, freshAxisPlaneDragState, lockedDimensionPlaneNormal, buildRadialGeometry, chordPassesNearCentre } from '../viewport/annotationLayout'
import { nextIdlePlaneLock, AXIS_LOCK_COLOR_NAMES } from './moveInput'

/** Rubber-band preview color — matches `fatLine.ts`'s `PREVIEW_LINE_STYLE`
 * blue used by the draw tools' own gesture previews. */
const PREVIEW_COLOR = 0x4d90ff

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
   * chain (arc-vs-full-circle, `isFullCircle`). */
  sketch: bigint
  /** The curve chain's own handle (`sketch_curve_geom`'s `curve` id) — lets
   * `isFullCircle` ask the kernel for every one of the chain's edges
   * directly (`sketch_curve_edges`), regardless of which specific point the
   * resolving snap named (a quadrant, the centre, or a plain rim point all
   * resolve the SAME curve handle). */
  curveHandle: bigint
}

/**
 * Resolve `snap` to the drawn circle/arc it names, or `null` if it names no
 * curve (a plain line/vertex/face snap, or a polygon chain with no captured
 * analytic geometry — `sketch_curve_geom` is circles/arcs only).
 */
function tryResolveCurve(wasmScene: WasmScene, snap: Snap): ResolvedCurve | null {
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
  | { kind: 'have-a'; aNode: { kind: number; id: bigint } | null; aPoint: V3; gesturePlane: DrawPlane | null }
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

  statusHint(): string {
    const lockNote =
      this.planeLock !== null
        ? ` Locked to the ${AXIS_LOCK_COLOR_NAMES[this.planeLock]} plane — same arrow or ArrowDown unlocks.`
        : ''
    switch (this.stage.kind) {
      case 'have-a':
        return `Click the second point.${lockNote}`
      case 'have-b':
        return `Drag out the dimension line, then click to place it — arrows lock the plane.${lockNote}`
      case 'have-curve': {
        const kind = this.stage.preferredKind ?? this._defaultRadialKind(this.stage)
        return `Drag out the leader, then click to place — Tab for ${kind === 'radius' ? 'diameter' : 'radius'}.`
      }
      default:
        return `Click a point to dimension from, or click a drawn circle/arc for a radius dimension.${lockNote}`
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

  capturingInput(): boolean {
    return this.stage.kind !== 'idle'
  }

  snapConstraint(ray?: Ray): { constraintPlane?: { point: V3; normal: V3 } } | null {
    if (this.stage.kind === 'have-a') {
      const plane = this.stage.gesturePlane
      if (plane !== null) return { constraintPlane: { point: plane.origin, normal: plane.normal } }
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
    const cursor: V3 = [snap.x, snap.y, snap.z]

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
        const prefix = cls.kind === 'diameter' ? 'Ø ' : 'R '
        this.onMeasurementCb(`${prefix}${formatLength(value)}`)
      } else if (cls.kind === 'linear') {
        const dist = length(sub(cls.bPoint, cls.aPoint))
        this.onMeasurementCb(formatLength(dist))
      } else {
        this.onMeasurementCb('')
      }
      return
    }

    if (this.stage.kind === 'have-a') {
      this._updatePreview(cursor)
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (snap === null) return
    const point: V3 = [snap.x, snap.y, snap.z]

    if (this.stage.kind === 'idle') {
      const curve = tryResolveCurve(this.wasmScene, snap)
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
      // An active arrow-key lock beats sketch-plane adoption (the draw
      // tools' own "lock overrides adoption" rule) — the probe simply
      // doesn't run while locked.
      const gesturePlane = this.planeLock !== null ? null : this._resolveGesturePlane(ray)
      this.stage = { kind: 'have-a', aNode: anchorNodeFromSnap(snap), aPoint: point, gesturePlane }
      this._updatePreview()
      return
    }

    if (this.stage.kind === 'have-a') {
      if (length(sub(point, this.stage.aPoint)) < 1e-9) return // same point twice — ignore
      this._axisDragState = freshAxisPlaneDragState() // a NEW baseline starts its drag unlatched
      this.stage = {
        kind: 'have-b',
        aNode: this.stage.aNode,
        aPoint: this.stage.aPoint,
        bNode: anchorNodeFromSnap(snap),
        bPoint: point,
        offset: [0, 0, 0],
        gesturePlane: this.stage.gesturePlane,
      }
      this._updatePreview()
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
      const prefix = this.stage.preferredKind === 'diameter' ? 'Ø ' : 'R '
      this.onMeasurementCb(`${prefix}${formatLength(value)}`)
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

  /** The kind the off-curve-into-space row of the disambiguation table would
   * default to right now (dimensions-playtest2.md §4: Radius for an arc,
   * Diameter for a full circle), ignoring any Tab override — used both to
   * seed Tab's first press and to describe the current default in
   * `statusHint`. */
  private _defaultRadialKind(stage: Extract<LinearStage, { kind: 'have-curve' }>): 'radius' | 'diameter' {
    return isFullCircle(this.wasmScene, stage.curve.sketch, stage.curve.curveHandle) ? 'diameter' : 'radius'
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
    const curve2 = tryResolveCurve(this.wasmScene, snap)
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
    const unidentified = snap.object === undefined && snap.instance === undefined && snap.sketch === undefined
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
      this.onToast("Drag away from the line to set the dimension's offset")
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
      this.onToast(`Couldn't create dimension: ${friendlyErrorText(err)}`)
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
      this.onToast(`Couldn't create radial dimension: ${friendlyErrorText(err)}`)
    }
    this.cancel()
  }

  /** Rebuild the live rubber-band preview from the current stage. */
  private _updatePreview(hoverPoint?: V3): void {
    this._clearPreview()
    let positions: number[] | null = null

    if (this.stage.kind === 'have-a' && hoverPoint !== undefined) {
      positions = [...this.stage.aPoint, ...hoverPoint]
    } else if (this.stage.kind === 'have-b') {
      const { aPoint, bPoint, offset } = this.stage
      const a1 = add(aPoint, offset)
      const b1 = add(bPoint, offset)
      positions = [...aPoint, ...a1, ...bPoint, ...b1, ...a1, ...b1]
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
    const mat = new THREE.LineBasicMaterial({ color: PREVIEW_COLOR, depthTest: false })
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
