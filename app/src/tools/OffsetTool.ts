/**
 * OffsetTool — SketchUp-style Offset: click a solid face or a sketch region,
 * drag inward/outward to preview a concentric copy of its boundary, click
 * (or type an exact distance) to commit.
 *
 * Gesture:
 *   1. Idle: hover; first click picks the target under the cursor —
 *      an object face (Path A, `pick_face`) or a sketch region (Path B,
 *      `pick_sketch_region`), the same two-path pick Push/Pull uses. A
 *      click landing ON a region's boundary edge (the region pick is
 *      interior-only) falls back to the edge pick and resolves the edge
 *      to its owning region, so one click on an edge OR the interior
 *      starts the gesture.
 *   2. Drag: the cursor's position on the target plane sets a SIGNED
 *      distance — negative inside the boundary (inset), positive outside
 *      (outset). The kernel computes the true offset loop each move
 *      (`sketch_offset_region_preview` / `offset_face_preview`); the tool
 *      renders it as viewport ephemera. An impossible distance (collapse)
 *      simply shows no loop.
 *   3. Second click (or a typed distance + Enter) commits:
 *      - region → `sketch_offset_region` inside a sketch gesture bracket
 *        (one undo step); both the original and offset regions stay
 *        extrudable.
 *      - face → `offset_face`, which imprints the loop like drawing on the
 *        face does (boss/recess workflow). Only inward offsets can land on
 *        a face; an outward commit is refused by the kernel with a toast.
 *   4. Esc cancels.
 *
 * The kernel owns truth: nothing partial ever reaches it, and every commit
 * is a single Scene mutation. Typed kernel errors surface as toasts.
 */

import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { rayPlaneIntersect, type V3 } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { signedOffsetDistance, decodeOffsetLoops, loopToSegmentPairs, boundaryContainsEdge } from './offsetMath'

/** Snap kinds whose point is a deliberate reference for the offset distance
 * (mirrors PushPull's set). `on-face` is excluded: it fires continuously
 * during a drag and would hijack the free drag. */
const HARD_SNAP_KINDS = new Set([
  'endpoint',
  'center',
  'quadrant',
  'tangent',
  'midpoint',
  'intersection',
  'on-edge',
  'on-guide',
  'on-axis',
])

export type OffsetTarget =
  | { kind: 'region'; sketchHandle: bigint; regionHandle: bigint }
  | { kind: 'face'; objectHandle: bigint; faceHandle: bigint }

export type OnOffsetCommit = (target: OffsetTarget) => void
export type OnFaceImprint = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'dragging'
      target: OffsetTarget
      /** The target's outer boundary loop, flat [x,y,z,…] world coords. */
      boundary: Float32Array
      /** The target plane (drag constraint + distance measurement). */
      planePoint: V3
      planeNormal: V3
      /** Last computed signed distance (negative = inward), or `null` while
       * the cursor has no in-plane position (fresh anchor, or a ray past the
       * plane's horizon) — a click then commits nothing. */
      distance: number | null
      /** The last VALID signed distance seen this drag: an off-plane
       * excursion nulls `distance` but never this, so typed entry keeps the
       * user's established inward/outward direction (the CircleTool
       * precedent of caching the last valid cursor). Click commits never
       * read it — only typed entry does, for its sign. */
      lastValidDistance: number | null
    }

export class OffsetTool implements Tool {
  readonly name = 'Offset'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    return this.stage.kind === 'idle'
      ? 'Click a face or a closed profile to offset its boundary.'
      : 'Drag inward or outward, click to commit — or type an exact distance.'
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnOffsetCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** VCB buffer — raw string being typed by the user. */
  private typed: string = ''

  /** The snap last seen on hover (CueLayer reads it). */
  lastSnap: Snap | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnOffsetCommit,
    onToast: OnToast,
    onFaceImprint: OnFaceImprint,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast
    this.onFaceImprint = onFaceImprint
    this.onMeasurementCb = onMeasurement
  }

  // ── Optional Tool interface extensions ────────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind === 'dragging'
  }

  /** While dragging, lock snapping to the target plane so the cursor never
   * snaps through the solid to occluded geometry (the CircleTool pattern). */
  snapConstraint(): {
    constraintPlane?: { point: V3; normal: V3 }
  } | null {
    if (this.stage.kind !== 'dragging') return null
    return {
      constraintPlane: { point: this.stage.planePoint, normal: this.stage.planeNormal },
    }
  }

  /** The current editing context (component-edit-parity.md phase A1). The
   *  getters below preserve the old `_activeContext` field's read semantics. */
  private _editContext: EditContext = { kind: 'top' }
  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
    this.cancel()
  }

  /** The entered OBJECT id, or null. When set, Offset only acts on that
   *  object's faces (scoped editing) — unchanged from the old field. */
  private get _activeContext(): bigint | null {
    return this._editContext.kind === 'object' ? this._editContext.id : null
  }

  /**
   * The entered component INSTANCE id, or null (component-edit-parity.md
   * phase A2). Offset's REGION path (`sketch_offset_region`) is pure
   * sketch-to-sketch geometry with no world-frame math (K1's own design
   * note), but reaching a def-owned sketch still needs the same routing
   * fix push/pull needed: `pick_sketch_region` only ever walks
   * `Document::sketch_ids()` (world-tree sketches), so `_pickTarget`'s
   * Path B calls `pick_sketch_region_in_instance` instead whenever this is
   * non-null — exactly like `PushPullTool`'s Path B. Offset's FACE path
   * (`offset_face`) has no `_in_component`/`_in_instance` wasm entry point,
   * unlike push/pull's — a member face is refused here with a clear hint
   * instead of reaching the kernel's raw `is_world` refusal (see
   * `_pickTarget`'s Path A).
   */
  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    this.lastSnap = snap
    if (this.stage.kind !== 'dragging') return

    const distance = this._dragDistance(snap, ray)
    this.stage = {
      ...this.stage,
      distance,
      lastValidDistance: distance ?? this.stage.lastValidDistance,
    }
    if (distance === null) {
      // The cursor has no position on the target plane (ray past the plane's
      // horizon in a perspective view — ordinary mid-drag). Never leave a
      // stale loop or readout on screen: clear both, like CircleTool does,
      // and let the null stage.distance make a commit click a no-op.
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
      return
    }
    this._drawPreview(distance)
    this._reportMeasurement(distance)
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'idle') {
      this._pickTarget(ray)
      return
    }

    // Second click: commit at the click's own distance. A click while the
    // cursor is off the plane (no live distance) commits nothing — the drag
    // stays open until the cursor is back over the plane or Esc cancels.
    const final = this._dragDistance(snap, ray) ?? this.stage.distance
    if (final === null) return
    this._commit(final)
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }
    if (this.stage.kind !== 'dragging') return

    if (ev.key === 'Enter') {
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitFromTyped(meters)
      }
      return
    }
    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(typedReadout(this.typed))
    }
  }

  cancel(): void {
    this.stage = { kind: 'idle' }
    this.typed = ''
    this._clearPreview()
    this.onMeasurementCb('')
    this.lastSnap = null
  }

  // ── Stage 1: pick ──────────────────────────────────────────────────────────

  private _pickTarget(ray: Ray): void {
    // Path A: nearest object face (scoped to the editing context, like
    // Push/Pull — inside a context only the entered object is editable).
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (pick !== undefined) {
      try {
        const objectHandle = pick.object()
        const instanceHandle = pick.instance()
        // Offset's face path (`offset_face`) has no `_in_component`/
        // `_in_instance` wasm entry point (component-edit-parity.md phase
        // A2 — unlike push/pull's) — refuse a member face with a clear hint
        // rather than reach the kernel's raw `is_world` refusal. ALWAYS
        // toast, regardless of context: the far more common case is a
        // top-level click on an ordinary placed instance (`_activeInstance
        // === null`), which used to hit this same branch and return with
        // zero feedback — no drag, no toast, nothing — unlike every other
        // refusal path in this tool and unlike PushPullTool's equivalent
        // (`_ineligibleFaceHint`), which always surfaces a toast.
        if (instanceHandle !== undefined) {
          this.onToast(
            'Offset can’t cut a component’s member face yet — try a sketch region instead, or explode the instance.',
          )
          return
        }
        if (this._activeContext === null || objectHandle === this._activeContext) {
          const faceHandle = pick.face()
          try {
            const boundary = this.wasmScene.face_boundary(objectHandle, faceHandle)
            const plane = this.wasmScene.face_plane(objectHandle, faceHandle)
            this._beginDrag(
              { kind: 'face', objectHandle, faceHandle },
              boundary,
              [plane[0], plane[1], plane[2]],
              [plane[3], plane[4], plane[5]],
            )
          } catch {
            // Degenerate/stale face — stay idle.
          }
          return
        }
      } finally {
        pick.free()
      }
    }

    // Path B: no object face — a sketch region (top-level OR instance-
    // context act — an instance context has its own def-owned regions too,
    // component-edit-parity.md phase A2). `pick_sketch_region` only ever
    // walks `Document::sketch_ids()` (world-tree sketches) — it can NEVER
    // find a region drawn inside a component's own definition, so an
    // instance context calls the `_in_instance` sibling instead, exactly
    // like `PushPullTool`'s identical Path B.
    if (this._activeContext !== null) return
    const activeInstance = this._activeInstance
    const regionPick =
      activeInstance !== null
        ? this.wasmScene.pick_sketch_region_in_instance(
            activeInstance,
            ray.origin[0], ray.origin[1], ray.origin[2],
            ray.direction[0], ray.direction[1], ray.direction[2],
          )
        : this.wasmScene.pick_sketch_region(
            ray.origin[0], ray.origin[1], ray.origin[2],
            ray.direction[0], ray.direction[1], ray.direction[2],
          )
    if (regionPick === undefined) {
      // The region pick is interior-only, so a click landing ON the visible
      // boundary line — exactly where Offset invites clicking — always
      // missed (maintainer playtest: "the first click is always swallowed").
      // Fall back to the edge pick (which has the proper pixel tolerance)
      // and resolve the edge to the region whose boundary carries it. This
      // fallback stays world-only — `pick_sketch_edge`/`sketch_edge_endpoints`
      // have no `_in_instance` sibling — so a def-owned sketch's BOUNDARY
      // line still misses inside an instance context; only the interior
      // click this finding tracks is fixed here. Its find is ALWAYS a WORLD
      // sketch (never pose-mapped below), even while `activeInstance` is
      // non-null — a nearby world sketch remains reachable this way while
      // editing an instance, same as before this fix.
      const viaEdge = this._pickRegionViaEdge(ray)
      if (viaEdge !== null) {
        this._beginRegionDrag(viaEdge.sketchHandle, viaEdge.regionHandle, false)
      }
      return
    }
    try {
      this._beginRegionDrag(regionPick.sketch(), regionPick.region(), activeInstance !== null)
    } finally {
      regionPick.free()
    }
  }

  /**
   * Start the drag on a picked sketch region (both region-pick arms).
   * `isDefOwned` — true only when `sketchHandle` came back from
   * `pick_sketch_region_in_instance` (so it is genuinely one of the active
   * instance's own definition-member sketches) — gates whether `boundary`/
   * the plane normal need mapping through the pose at all: the edge-pick
   * fallback above is world-only, so a region it finds is ALREADY in world
   * space and must NOT be re-mapped, even while `activeInstance` is
   * non-null (editing an instance doesn't stop a nearby world sketch from
   * being reachable this way).
   */
  private _beginRegionDrag(sketchHandle: bigint, regionHandle: bigint, isDefOwned: boolean): void {
    try {
      let boundary = this.wasmScene.region_boundary(sketchHandle, regionHandle)
      if (boundary.length < 9) return
      // Anchor the plane at the boundary's first vertex; the normal comes
      // from the sketch's own plane (sketches on any plane — Phase 1) so a
      // rotated sketch still measures its offset in-plane.
      const plane = this.wasmScene.sketch_plane(sketchHandle)
      if (plane === undefined) return // stale handle — stay idle
      let normal: V3 = [plane[3], plane[4], plane[5]]
      const activeInstance = this._activeInstance
      if (isDefOwned && activeInstance !== null) {
        // A def-owned sketch's `region_boundary`/`sketch_plane` both answer
        // in DEFINITION-LOCAL space (component-edit-parity.md phase A2) —
        // `Stage['boundary']` is documented WORLD coords (this drag's own
        // preview/measurement math needs it), so every boundary vertex is
        // mapped through the pose here (exact — a point maps unambiguously
        // under any invertible affine, unlike a normal or a scalar length).
        // The normal is mapped the same approximate way PushPullTool's
        // Path B does (linear part, re-normalized): exact for rotation/
        // uniform-scale/mirror/translation, and even a non-uniform-scale
        // pose can only skew the drag axis/preview, never the actual
        // commit — `sketch_offset_region` is pure sketch-local geometry
        // with no world-frame math at all.
        const pose = this.wasmScene.instance_pose(activeInstance)
        if (pose === undefined) return // stale instance — stay idle
        const mapped = new Float32Array(boundary.length)
        for (let i = 0; i + 2 < boundary.length; i += 3) {
          mapped[i] = pose[0] * boundary[i] + pose[1] * boundary[i + 1] + pose[2] * boundary[i + 2] + pose[3]
          mapped[i + 1] = pose[4] * boundary[i] + pose[5] * boundary[i + 1] + pose[6] * boundary[i + 2] + pose[7]
          mapped[i + 2] = pose[8] * boundary[i] + pose[9] * boundary[i + 1] + pose[10] * boundary[i + 2] + pose[11]
        }
        boundary = mapped
        const [nx, ny, nz] = normal
        const rnx = pose[0] * nx + pose[1] * ny + pose[2] * nz
        const rny = pose[4] * nx + pose[5] * ny + pose[6] * nz
        const rnz = pose[8] * nx + pose[9] * ny + pose[10] * nz
        const len = Math.hypot(rnx, rny, rnz)
        if (len <= 1e-12) return
        normal = [rnx / len, rny / len, rnz / len]
      }
      this._beginDrag(
        { kind: 'region', sketchHandle, regionHandle },
        boundary,
        [boundary[0], boundary[1], boundary[2]],
        normal,
      )
    } catch {
      // Stale region — stay idle.
    }
  }

  /**
   * Resolve a click that landed on a sketch EDGE to the region whose outer
   * boundary contains that edge. Returns the first matching region in the
   * kernel's (deterministic) region order — for the common case, a lone
   * closed profile, that is the only one; for a shared boundary edge either
   * neighbor is a reasonable pick and an interior click disambiguates.
   * Null when nothing usable is under the cursor (open-line edge, stale
   * handles, no sketch at all).
   */
  private _pickRegionViaEdge(
    ray: Ray,
  ): { sketchHandle: bigint; regionHandle: bigint } | null {
    const edgePick = this.wasmScene.pick_sketch_edge(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (edgePick === undefined) return null
    try {
      const sketchHandle = edgePick.sketch()
      const endpoints = this.wasmScene.sketch_edge_endpoints(sketchHandle, edgePick.edge())
      if (endpoints === undefined) return null
      try {
        for (const regionHandle of Array.from(this.wasmScene.sketch_regions(sketchHandle))) {
          const boundary = this.wasmScene.region_boundary(sketchHandle, regionHandle)
          if (boundaryContainsEdge(boundary, endpoints)) {
            return { sketchHandle, regionHandle }
          }
        }
      } catch {
        // A region vanished between the queries — treat as a miss.
      }
      return null
    } finally {
      edgePick.free()
    }
  }

  private _beginDrag(
    target: OffsetTarget,
    boundary: Float32Array,
    planePoint: V3,
    planeNormal: V3,
  ): void {
    this.typed = ''
    this.stage = {
      kind: 'dragging',
      target,
      boundary,
      planePoint,
      planeNormal,
      distance: null,
      lastValidDistance: null,
    }
  }

  // ── Stage 2: drag ──────────────────────────────────────────────────────────

  /** The signed offset distance for the current cursor: the in-plane
   * distance from the target boundary, negative inside it. A deliberate
   * inference snap contributes its exact point (projected to the plane), so
   * the offset distance can lock onto real geometry; free drags follow the
   * ray-plane intersection. */
  private _dragDistance(snap: Snap | null, ray: Ray): number | null {
    if (this.stage.kind !== 'dragging') return null
    const { boundary, planePoint, planeNormal } = this.stage

    let p: V3 | null = null
    if (snap !== null && HARD_SNAP_KINDS.has(snap.kind)) {
      // Project the snapped point onto the target plane.
      const d =
        (snap.x - planePoint[0]) * planeNormal[0] +
        (snap.y - planePoint[1]) * planeNormal[1] +
        (snap.z - planePoint[2]) * planeNormal[2]
      p = [
        snap.x - planeNormal[0] * d,
        snap.y - planeNormal[1] * d,
        snap.z - planeNormal[2] * d,
      ]
    } else {
      p = rayPlaneIntersect(ray.origin, ray.direction, planePoint, planeNormal)
    }
    if (p === null) return null
    return signedOffsetDistance(p, boundary, planeNormal)
  }

  // ── Preview ────────────────────────────────────────────────────────────────

  private _drawPreview(distance: number): void {
    this._clearPreview()
    if (this.stage.kind !== 'dragging' || Math.abs(distance) < 1e-6) return
    const { target } = this.stage

    // The kernel computes the true offset loops; an impossible distance
    // (collapse, degenerate) throws and simply draws nothing — the vanished
    // preview IS the feedback, and a commit would surface the typed error.
    let loops: number[][]
    try {
      if (target.kind === 'region') {
        const data = this.wasmScene.sketch_offset_region_preview(
          target.sketchHandle,
          target.regionHandle,
          distance,
        )
        loops = decodeOffsetLoops(data)
      } else {
        const data = this.wasmScene.offset_face_preview(
          target.objectHandle,
          target.faceHandle,
          distance,
        )
        loops = [Array.from(data)]
      }
    } catch {
      return
    }

    for (const loop of loops) {
      if (loop.length < 9) continue
      this.preview.add(makeFatSegments(loopToSegmentPairs(loop), PREVIEW_LINE_STYLE))
    }
  }

  private _clearPreview(): void {
    this.preview.traverse((child) => {
      disposeFatSegments(child)
      if (child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    })
    this.preview.clear()
  }

  // ── Commit ─────────────────────────────────────────────────────────────────

  /** Commit from the typed VCB buffer: magnitude from the typed length,
   * sign from the drag direction — the live one, or the last valid one when
   * the cursor happens to be off the plane at Enter (an excursion must not
   * discard the side the user already established). With no drag direction
   * at all (no movement yet), a face offset defaults inward (the only
   * direction that can land on the face) and a region offset defaults
   * outward. */
  private _commitFromTyped(meters: number): void {
    if (this.stage.kind !== 'dragging') return
    const { target, distance, lastValidDistance } = this.stage
    const directional = distance ?? lastValidDistance
    const sign =
      directional !== null && directional < 0
        ? -1
        : directional !== null && directional > 0
          ? 1
          : target.kind === 'face'
            ? -1
            : 1
    this._commit(Math.abs(meters) * sign)
  }

  private _commit(distance: number): void {
    if (this.stage.kind !== 'dragging') return
    const { target } = this.stage

    if (Math.abs(distance) < 1e-6) {
      this.onToast('Move more before committing the offset')
      return
    }

    this.stage = { kind: 'idle' }
    this.typed = ''
    this._clearPreview()
    this.onMeasurementCb('')

    try {
      if (target.kind === 'region') {
        // One undo step: gesture-bracket the single offset mutation. A
        // failed offset leaves the gesture unchanged, so ending it records
        // nothing.
        this.wasmScene.sketch_begin_gesture(target.sketchHandle)
        try {
          const report = this.wasmScene.sketch_offset_region(
            target.sketchHandle,
            target.regionHandle,
            distance,
          )
          report.free()
        } finally {
          this.wasmScene.sketch_end_gesture(target.sketchHandle)
        }
        this.onCommit(target)
      } else {
        this.wasmScene.offset_face(target.objectHandle, target.faceHandle, distance)
        this.onFaceImprint(target.objectHandle)
      }
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
    }
  }

  /** Live measurement: the typed buffer once the user starts typing,
   * otherwise the signed live distance (inward reads negative). */
  private _reportMeasurement(distance: number): void {
    if (this.typed !== '') {
      this.onMeasurementCb(typedReadout(this.typed))
      return
    }
    this.onMeasurementCb(formatLength(distance))
  }
}
