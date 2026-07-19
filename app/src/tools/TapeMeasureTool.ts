/**
 * TapeMeasureTool — SketchUp-style Tape Measure, scoped to what the
 * current kernel/UI support: live distance readout, parallel construction
 * guide lines (from an existing edge), and standalone guide points.
 *
 * Gesture:
 *   1. First click:
 *      - On an edge (snap.elementKind === 'edge' for a live world Object,
 *        or 'sketch-edge' for a committed sketch line) → PARALLEL-GUIDE
 *        mode. Remembers the edge's two endpoints (via
 *        `wasmScene.edge_endpoints` / `sketch_edge_endpoints`) and the
 *        picked point on it.
 *      - On a world axis or an existing guide line ('on-axis'/'on-guide'
 *        snaps, which carry the analytic line's direction) → the same
 *        PARALLEL-GUIDE mode, sourced from the infinite analytic line.
 *      - Anywhere else → MEASURE mode. Remembers the picked point as P0.
 *   2. Pointer move previews:
 *      - Parallel mode: a dashed guide line through (edgePoint + offset)
 *        along the edge direction, where `offset` is the component of
 *        (cursor − edgePoint) perpendicular to the edge direction (so
 *        dragging the cursor off the edge "pulls" the guide sideways, as in
 *        SketchUp).
 *      - Measure mode: a dashed segment from P0 to the cursor, with a live
 *        `formatLength` readout via the measurement callback.
 *   3. Second click commits:
 *      - Parallel mode → `add_guide_line(origin, direction)`.
 *      - Measure mode, cursor on real geometry → just finalizes the readout
 *        (SketchUp does the same: measuring between two existing points
 *        doesn't drop a guide).
 *      - Measure mode, cursor in empty space → `add_guide_point(point)`.
 *   4. VCB numeric entry (typed digits while a stage is active) commits an
 *      exact typed distance along the current direction, mirroring MoveTool.
 *   5. Esc cancels the current stage; `cancel()` clears all preview state.
 *
 * Sketches on any plane (the sketch-planes design §6 bullet 2): if the
 * FIRST pick of a gesture hovers a committed sketch whose plane is
 * non-ground (`pick_sketch` + `planeFromSketch` — the same helpers the draw
 * tools use), OR an idle arrow-key plane lock is active, the gesture freezes
 * that plane (`_gesturePlane`) and `snapConstraint()` returns it for the
 * REST of the gesture — so a guide/measurement started on a tilted sketch
 * stays in that sketch's plane instead of resolving to the ground fallback
 * and refusing (rule 4). The idle lock uses the same `nextIdlePlaneLock`
 * (arrow keys) as the draw tools — TapeMeasure doesn't otherwise use arrow
 * keys, so it drops in with no conflict (unlike Protractor, which already
 * owns all four arrows for its own plane lock — see ProtractorTool.ts).
 * In PARALLEL-guide mode specifically, the picked edge wins over a frozen
 * plane that disagrees with it: if the edge (from `_tryResolveEdge`) doesn't
 * actually lie in `_gesturePlane` — an idle arrow-key lock chosen before any
 * edge was known can point anywhere — the gesture drops the constraint and
 * behaves as legacy unconstrained parallel-guide (`_edgeLiesInPlane`).
 *
 * Explicitly OUT of scope for this slice (see ROADMAP):
 *   - Protractor (angle guides) —, a separate tool.
 *   - Picking an individual existing guide to delete it — needs per-entity
 *     pick/select for guides, not built yet. Edit ▸ Delete Guide Lines
 *     (delete_all_guides) is the only deletion path for now.
 *   - Parallel guides from an edge on INSTANCED geometry: `edge_endpoints`
 *     only resolves live world Objects (instances are placed copies of a
 *     shared definition, not their own object), so an edge snap on instanced
 *     geometry simply falls back to measure mode (object-space coordinates
 *     would need the instance pose composed in, which is unsupported here).
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { editLengthBuffer, isLengthInputKey, pointAlong, nextIdlePlaneLock, AXIS_LOCK_COLOR_NAMES } from './moveInput'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { planeFromSketch, axisDrawPlane, SketchPickCache, GROUND_PLANE_EPS, type DrawPlane } from './drawPlane'

/** Whether a snap landed on real picked geometry, as opposed to a broad
 * empty-space fallback: 'ground' (ray∩ground) or 'plane' (ray∩constraint
 * plane — sketches on any plane, Phase 1). Guides anchored to geometry
 * survive re-inference; free-space landings do not count. */
function snapOnGeometry(snap: Snap): boolean {
  return snap.kind !== 'ground' && snap.kind !== 'plane'
}

export type OnGuideCreated = () => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Construction guide color — matches SceneRenderer's GUIDE_COLOR. */
const GUIDE_PREVIEW_COLOR = 0x9933cc
/** Half-length of the previewed parallel-guide line (meters). */
const GUIDE_HALF_LENGTH = 50

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'parallel'
      /** A point on the source edge, in world space (the pick that started this gesture). */
      edgePoint: [number, number, number]
      /** Unit direction of the source edge. */
      edgeDir: [number, number, number]
      /** Last computed guide origin (edgePoint + perpendicular offset). */
      origin: [number, number, number]
    }
  | {
      kind: 'measure'
      /** First picked point, in world space. */
      p0: [number, number, number]
      /** Last cursor point (snapped), in world space. */
      p1: [number, number, number]
      /** Whether the cursor is currently resting on real geometry (vs. empty space). */
      onGeometry: boolean
    }

/** v − (v·d)d for unit d: the component of v perpendicular to d. */
function perpComponent(
  v: [number, number, number],
  d: [number, number, number],
): [number, number, number] {
  const dot = v[0] * d[0] + v[1] * d[1] + v[2] * d[2]
  return [v[0] - dot * d[0], v[1] - dot * d[1], v[2] - dot * d[2]]
}

export class TapeMeasureTool implements Tool {
  readonly name = 'Tape Measure'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    switch (this.stage.kind) {
      case 'parallel':
        return 'Click to place the parallel guide — or type an exact offset.'
      case 'measure':
        return 'Click the second point to read the distance — or type an exact distance to drop a guide there.'
      default:
        if (this.idlePlaneLock !== null) {
          return `Locked to the ${AXIS_LOCK_COLOR_NAMES[this.idlePlaneLock]} plane — click to start; same arrow or Esc unlocks.`
        }
        return 'Click a point to measure from — or click an edge to drop a parallel guide.'
    }
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onGuideCreated: OnGuideCreated
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** VCB buffer — raw string being typed by the user. */
  private typed: string = ''

  /** THREE.js LineSegments for the preview guide/segment. */
  private previewLine: THREE.LineSegments | null = null

  /** Per-pointer-event `pick_sketch` memo — see `SketchPickCache` in drawPlane.ts. */
  private readonly _sketchPickCache = new SketchPickCache()

  /** The plane the CURRENT gesture is frozen to (sketches on any plane,
   *  design §6 bullet 2) — resolved once at the first click, from either the
   *  idle arrow-key lock or a hovered non-ground sketch; null for an
   *  unconstrained (ground/free-space) gesture. Cleared back to null by
   *  `_resetToIdle()`. */
  private _gesturePlane: DrawPlane | null = null

  /** Idle plane lock (mirrors the draw tools' — design §5.2/§6 bullet 2):
   *  while FULLY idle, an arrow key locks the future plane's NORMAL to a
   *  world axis; the same arrow again, or Escape/ArrowDown, clears it.
   *  Consumed by the first click, which freezes `_gesturePlane` through the
   *  clicked point. Survives a completed gesture (cleared only by
   *  `cancel()`). */
  private idlePlaneLock: 0 | 1 | 2 | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onGuideCreated: OnGuideCreated,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onGuideCreated = onGuideCreated
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  // ── Optional Tool interface extensions ─────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind !== 'idle'
  }

  /**
   * Constrain snapping to the gesture's frozen plane (design §6 bullet 2):
   * - Mid-gesture: `_gesturePlane`, if the first click resolved one (else
   *   unconstrained — today's behavior).
   * - Idle: the idle lock is FREE (no constraint — the locked plane is
   *   derived FROM the first click, same rationale as the draw tools);
   *   absent a lock, a top-level hover over a non-ground sketch previews
   *   its plane so the first click lands precisely on it.
   */
  snapConstraint(ray: Ray): { constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null {
    if (this.stage.kind !== 'idle') {
      if (this._gesturePlane !== null) {
        return { constraintPlane: { point: this._gesturePlane.origin, normal: this._gesturePlane.normal } }
      }
      return null
    }

    if (this.idlePlaneLock !== null) return null

    const handle = this._sketchPickCache.pickFor(this.wasmScene, ray)
    if (handle !== null) {
      const plane = planeFromSketch(this.wasmScene, handle)
      if (plane !== null && !plane.ground) {
        return { constraintPlane: { point: plane.origin, normal: plane.normal } }
      }
    }
    return null
  }

  // ── Tool interface ──────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'parallel') {
      const cursor: [number, number, number] = [snap.x, snap.y, snap.z]
      const rel: [number, number, number] = [
        cursor[0] - this.stage.edgePoint[0],
        cursor[1] - this.stage.edgePoint[1],
        cursor[2] - this.stage.edgePoint[2],
      ]
      const offset = perpComponent(rel, this.stage.edgeDir)
      const origin: [number, number, number] = [
        this.stage.edgePoint[0] + offset[0],
        this.stage.edgePoint[1] + offset[1],
        this.stage.edgePoint[2] + offset[2],
      ]
      this.stage.origin = origin
      this._updatePreviewLine()
      // No numeric readout for parallel mode — SketchUp shows the offset
      // distance from the original edge; report that here.
      const offsetDist = Math.sqrt(offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2])
      this._reportOffsetOrTyped(offsetDist)
      return
    }

    if (this.stage.kind === 'measure') {
      this.stage.p1 = [snap.x, snap.y, snap.z]
      this.stage.onGeometry = snapOnGeometry(snap)
      this._updatePreviewLine()
      this._reportDistanceOrTyped(this.stage.p0, this.stage.p1)
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      // Freeze the gesture's plane (design §6 bullet 2), BEFORE branching
      // into parallel/measure mode, so it constrains snapping for either.
      this._gesturePlane = this._resolveGesturePlane(ray, [snap.x, snap.y, snap.z])

      const edge = this._tryResolveEdge(snap)
      if (edge !== null) {
        const { edgePoint, edgeDir } = edge
        // Blocker 2 (frozen plane vs. picked edge, adversarial review): the
        // EDGE wins. A frozen gesture plane only makes sense for parallel
        // mode if the source edge actually lies in it (the hover-adopted
        // tilted-sketch case); an idle arrow-key lock is chosen before any
        // edge is known and can disagree with whatever edge ends up picked.
        // If it does, `perpComponent` (below) mixes an off-plane edge
        // direction with an on-plane-constrained cursor snap and drifts the
        // guide origin off the frozen plane — so drop the constraint for
        // this gesture and fall back to legacy unconstrained behavior.
        if (this._gesturePlane !== null && !this._edgeLiesInPlane(edgePoint, edgeDir, this._gesturePlane)) {
          this._gesturePlane = null
        }
        this.stage = { kind: 'parallel', edgePoint, edgeDir, origin: edgePoint }
        this._updatePreviewLine()
        return
      }

      const p0: [number, number, number] = [snap.x, snap.y, snap.z]
      this.stage = { kind: 'measure', p0, p1: p0, onGeometry: snapOnGeometry(snap) }
      this._updatePreviewLine()
      return
    }

    if (this.stage.kind === 'parallel') {
      this._commitParallelGuide(this.stage.origin, this.stage.edgeDir)
      return
    }

    if (this.stage.kind === 'measure') {
      this._commitMeasure(this.stage.p0, [snap.x, snap.y, snap.z], snapOnGeometry(snap))
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      // Idle with an active plane lock: Escape clears the lock FIRST — only
      // a second Escape (already idle, unlocked) is a no-op cancel (mirrors
      // the draw tools' idle plane lock — design §6 bullet 2).
      if (this.stage.kind === 'idle' && this.idlePlaneLock !== null) {
        this.idlePlaneLock = null
        return
      }
      // Aborting an in-progress gesture keeps the plane lock: the lock is an
      // idle aiming choice, cleared only by an idle Escape or toggle
      // (parity with the draw tools).
      const lock = this.idlePlaneLock
      this.cancel()
      this.idlePlaneLock = lock
      return
    }

    if (this.stage.kind === 'idle') {
      // Idle plane lock via arrow keys (design §6 bullet 2) — consumed by
      // neither hover nor preview, only by the next first click.
      if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
        this.idlePlaneLock = nextIdlePlaneLock(this.idlePlaneLock, ev.key)
      }
      return
    }

    if (ev.key === 'Enter') {
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitFromTyped(meters)
      }
      return
    }

    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this._typedReadout())
    }
  }

  /** The typed-buffer readout, suffixed for metric formats (imperial tokens
   * like `'`/`"` are already visible in the buffer itself). */
  private _typedReadout(): string {
    return typedReadout(this.typed)
  }

  cancel(): void {
    this.idlePlaneLock = null
    this._resetToIdle()
  }

  /**
   * Resolve the plane the gesture starting at `clickedPoint` should freeze
   * to (design §6 bullet 2): an active idle lock (through the clicked
   * point) beats sketch-hover adoption, mirroring the draw tools'
   * `_resolveClickTarget`. Returns null for ground/free-space (no
   * constraint — today's behavior).
   */
  private _resolveGesturePlane(ray: Ray, clickedPoint: [number, number, number]): DrawPlane | null {
    if (this.idlePlaneLock !== null) {
      const plane = axisDrawPlane(this.idlePlaneLock, clickedPoint)
      return plane.ground ? null : plane
    }
    const handle = this._sketchPickCache.pickFor(this.wasmScene, ray)
    if (handle !== null) {
      const plane = planeFromSketch(this.wasmScene, handle)
      if (plane !== null && !plane.ground) return plane
    }
    return null
  }

  /**
   * True iff the line through `point` along unit `dir` lies IN `plane`
   * (Blocker 2): the direction must be perpendicular to the plane's normal,
   * AND the point itself must sit on the plane. Both checked against the
   * same 1e-9 tolerance `drawPlane.ts` uses for plane membership
   * (`GROUND_PLANE_EPS`).
   */
  private _edgeLiesInPlane(
    point: [number, number, number],
    dir: [number, number, number],
    plane: DrawPlane,
  ): boolean {
    const dirDotN = dir[0] * plane.normal[0] + dir[1] * plane.normal[1] + dir[2] * plane.normal[2]
    if (Math.abs(dirDotN) > GROUND_PLANE_EPS) return false
    const rel: [number, number, number] = [
      point[0] - plane.origin[0],
      point[1] - plane.origin[1],
      point[2] - plane.origin[2],
    ]
    const offset = rel[0] * plane.normal[0] + rel[1] * plane.normal[1] + rel[2] * plane.normal[2]
    return Math.abs(offset) <= GROUND_PLANE_EPS
  }

  // ── Private helpers ─────────────────────────────────────────────────────

  /**
   * Resolve a snap to a reference line (a point on it + its unit direction),
   * or null if the snap carries no line to be parallel to.
   *
   * Three sources qualify:
   * - a live world-Object edge (`elementKind === 'edge'`),
   * - a committed sketch edge (`elementKind === 'sketch-edge'`),
   * - a world axis or existing guide line (`kind === 'on-axis'` /
   *   `'on-guide'`): these carry no element handle at all — the kernel
   *   resolves them ANALYTICALLY (an infinite line, camera-independent) and
   *   hands back the on-line point plus the line's direction on the snap
   *   itself, which is everything parallel mode needs. Note the rendered
   *   axis geometry is irrelevant here: its per-frame clipped extent is a
   *   draw concern, never what the snap resolves against.
   *
   * An `endpoint` snap is a vertex, not a line, so it intentionally falls
   * through to measure mode (a simplification — SketchUp lets you start a
   * parallel guide from a vertex-snapped point on an edge too, but
   * distinguishing "vertex that happens to sit on an edge" needs more
   * inference-engine plumbing than this slice adds).
   */
  private _tryResolveEdge(
    snap: Snap,
  ): { edgePoint: [number, number, number]; edgeDir: [number, number, number] } | null {
    if ((snap.kind === 'on-axis' || snap.kind === 'on-guide') && snap.direction !== undefined) {
      const [dx, dy, dz] = snap.direction
      const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
      if (len < 1e-9) return null // degenerate direction — fall back
      return {
        edgePoint: [snap.x, snap.y, snap.z],
        edgeDir: [dx / len, dy / len, dz / len],
      }
    }

    let endpoints: Float64Array | number[] | undefined
    if (snap.elementKind === 'edge' && snap.object !== undefined && snap.element !== undefined) {
      endpoints = this.wasmScene.edge_endpoints(snap.object, snap.element)
    } else if (
      snap.elementKind === 'sketch-edge' &&
      snap.sketch !== undefined &&
      snap.element !== undefined
    ) {
      // A committed sketch line works as a guide reference too — the most
      // common case: a parallel guide off a just-drawn rectangle's edge.
      endpoints = this.wasmScene.sketch_edge_endpoints(snap.sketch, snap.element)
    }
    if (endpoints === undefined) return null // stale/consumed — fall back

    const [ax, ay, az, bx, by, bz] = endpoints
    const dx = bx - ax, dy = by - ay, dz = bz - az
    const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
    if (len < 1e-9) return null // degenerate edge

    return {
      edgePoint: [snap.x, snap.y, snap.z],
      edgeDir: [dx / len, dy / len, dz / len],
    }
  }

  private _commitParallelGuide(origin: [number, number, number], dir: [number, number, number]): void {
    try {
      this.wasmScene.add_guide_line(origin[0], origin[1], origin[2], dir[0], dir[1], dir[2])
      this.onGuideCreated()
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err)
      this.onToast(`Couldn't create guide line: ${raw}`)
    }
    this._resetToIdle()
  }

  /**
   * Commit a measure-mode gesture. If the endpoint rests on real geometry,
   * SketchUp only finalizes the readout (no guide is created, since both ends
   * are already well-defined points). In empty space, drop a guide point so
   * the measured point is preserved as a construction reference.
   */
  private _commitMeasure(
    p0: [number, number, number],
    p1: [number, number, number],
    onGeometry: boolean,
  ): void {
    const dx = p1[0] - p0[0], dy = p1[1] - p0[1], dz = p1[2] - p0[2]
    const dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    this.onMeasurementCb(formatLength(dist))

    if (!onGeometry) {
      try {
        this.wasmScene.add_guide_point(p1[0], p1[1], p1[2])
        this.onGuideCreated()
      } catch (err) {
        const raw = err instanceof Error ? err.message : String(err)
        this.onToast(`Couldn't create guide point: ${raw}`)
      }
    }
    this._resetToIdle()
  }

  /** Commit an exact typed distance (Enter in the VCB), per stage. */
  private _commitFromTyped(dist: number): void {
    if (this.stage.kind === 'parallel') {
      const { edgePoint, edgeDir, origin } = this.stage
      // Signed offset direction: from edgePoint toward the current origin
      // (i.e. whichever side of the edge the cursor is currently on).
      const rel: [number, number, number] = [
        origin[0] - edgePoint[0],
        origin[1] - edgePoint[1],
        origin[2] - edgePoint[2],
      ]
      const relLen = Math.sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2])
      if (relLen < 1e-9) {
        // No offset direction yet (cursor sitting on the edge) — nothing to commit.
        this._resetToIdle()
        return
      }
      const dir: [number, number, number] = [rel[0] / relLen, rel[1] / relLen, rel[2] / relLen]
      const newOrigin = pointAlong(edgePoint, dir, dist)
      this._commitParallelGuide(newOrigin, edgeDir)
      return
    }

    if (this.stage.kind === 'measure') {
      const { p0, p1 } = this.stage
      const rel: [number, number, number] = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]]
      const relLen = Math.sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2])
      const dir: [number, number, number] = relLen < 1e-9 ? [1, 0, 0] : [rel[0] / relLen, rel[1] / relLen, rel[2] / relLen]
      const endpoint = pointAlong(p0, dir, dist)
      // Typed-exact endpoints are, by definition, not resting on picked
      // geometry — always drop a guide point so the typed distance is preserved.
      this._commitMeasure(p0, endpoint, false)
    }
  }

  private _resetToIdle(): void {
    this.stage = { kind: 'idle' }
    this.typed = ''
    // The frozen gesture plane is stale either way (committed or aborted) —
    // the NEXT gesture re-resolves it at its own first click. The idle lock
    // itself is NOT cleared here — it survives a completed gesture, same as
    // the draw tools; only `cancel()` (idle Escape / explicit reset) clears it.
    this._gesturePlane = null
    this._clearPreviewLine()
    this.onMeasurementCb('')
  }

  private _reportOffsetOrTyped(offsetDist: number): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    this.onMeasurementCb(formatLength(offsetDist))
  }

  private _reportDistanceOrTyped(p0: [number, number, number], p1: [number, number, number]): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    const dx = p1[0] - p0[0], dy = p1[1] - p0[1], dz = p1[2] - p0[2]
    this.onMeasurementCb(formatLength(Math.sqrt(dx * dx + dy * dy + dz * dz)))
  }

  /**
   * Rebuild the dashed preview line in the preview group, dispatched by
   * stage: a long dashed line through (origin, edgeDir) in parallel mode, or
   * a dashed segment from p0 to p1 in measure mode. Removes the previous
   * preview (if any) first.
   */
  private _updatePreviewLine(): void {
    this._clearPreviewLine()

    // The preview is a solid line (the placed guide renders dashed). A
    // screen-constant manual dash here would just duplicate SceneRenderer's
    // dash logic for a transient overlay; a thin solid line reads clearly and
    // avoids the metre-sized-dash problem entirely.
    let pts: Float32Array
    if (this.stage.kind === 'parallel') {
      const { origin, edgeDir } = this.stage
      const nx = edgeDir[0] * GUIDE_HALF_LENGTH
      const ny = edgeDir[1] * GUIDE_HALF_LENGTH
      const nz = edgeDir[2] * GUIDE_HALF_LENGTH
      pts = new Float32Array([
        origin[0] - nx, origin[1] - ny, origin[2] - nz,
        origin[0] + nx, origin[1] + ny, origin[2] + nz,
      ])
    } else if (this.stage.kind === 'measure') {
      const { p0, p1 } = this.stage
      pts = new Float32Array([p0[0], p0[1], p0[2], p1[0], p1[1], p1[2]])
    } else {
      return
    }

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
    const mat = new THREE.LineBasicMaterial({ color: GUIDE_PREVIEW_COLOR, depthTest: false })
    const line = new THREE.LineSegments(geo, mat)
    this.preview.add(line)
    this.previewLine = line
  }

  private _clearPreviewLine(): void {
    if (this.previewLine === null) return
    this.previewLine.geometry.dispose()
    if (this.previewLine.material instanceof THREE.Material) {
      this.previewLine.material.dispose()
    }
    this.preview.remove(this.previewLine)
    this.previewLine = null
  }
}
