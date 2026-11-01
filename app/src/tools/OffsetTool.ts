/**
 * OffsetTool — SketchUp-style Offset: click a region of the drawing on a
 * solid's face, a sketch, or the ground, drag inward/outward to preview a
 * concentric copy of its boundary, click (or type an exact distance) to
 * commit.
 *
 * The plane is the drafting context (docs/design/unified-drafting.md): the
 * tool picks the plane under the cursor (the face struck, else the free
 * sketch's plane — the ground included), reads that plane's
 * `plane_units_json` listing, and works on the REGION the ray lands in. A
 * click ON a region's boundary edge (the fill pick is interior-only) resolves
 * the edge to the region it bounds, so one click on an edge OR the interior
 * starts the gesture.
 *
 * Gesture:
 *   1. Idle: hover; first click picks the region under the cursor.
 *   2. Drag: the cursor's position on the region's plane sets a SIGNED
 *      distance — negative inside the boundary (inset), positive outside
 *      (outset). The ghost is the kernel's exact offset of the region
 *      (`offset_region_preview`); an impossible distance (collapse) shows none.
 *   3. Second click (or a typed distance + Enter) commits ONE draft on the
 *      region's context: `draft_offset_region(regionKey, distance)` — one
 *      undo step, on faces and on the ground alike. An outward offset past a
 *      face's edge is accepted: the loop splits into a face part and a free
 *      part. Concentric circles keep their analytic claim in the kernel.
 *   4. Esc cancels.
 *
 * The kernel owns truth: nothing partial ever reaches it, and every commit
 * is a single draft. Typed kernel errors surface as toasts.
 */

import { t } from '@lingui/core/macro'
import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { rayPlaneIntersect, type V3 } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { RetypeWindow, idleRetypeCapturesKey, retypeStaleMessage } from './retypeWindow'
import { displayedLength } from './displayedValue'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { signedOffsetDistance, decodeOffsetLoops, loopToSegmentPairs } from './offsetMath'
import { runDraft, refreshAfterDraft } from './draftCommit'
import { defaultFaceEligible, ineligibleFaceHint, sessionIsOpen, EDIT_CONTEXT_HINT_CODE, type FaceEligible } from './faceDraw'
import { ctxOfFace, ctxOfSketch, keyToId, type PlaneUnitsScene } from './planeUnits'
import { regionUnderRay, regionOfFreeProfile, regionOfSegment, type PickedRegion } from './regionPick'

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

/** The region an offset works on: the plane context it lies in (the exact
 *  string `plane_units_json` listed it under), its listing key, and whether
 *  it lies over a solid's face (an offset there defaults inward). */
export interface OffsetTarget {
  ctx: string
  regionKey: string
  onFace: boolean
}

/** World tolerance, as a fraction of the ray depth, within which a click
 *  counts as ON a drawn boundary line (about eight pixels at the default
 *  view). */
const EDGE_PICK_FRACTION = 0.01

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
      /** The region's rings (world), for the ghost loop. */
      region: PickedRegion
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

/**
 * The offset just committed, kept in a `RetypeWindow` (retypeWindow.ts) so
 * a distance typed AFTER the commit click redoes it at that distance — a
 * positive value keeps the committed inward/outward direction, a negative
 * one flips it.
 */
type RetypeSpec = { target: OffsetTarget; distance: number }

export class OffsetTool implements Tool {
  readonly name = 'Offset'
  /** Click-drag-release works as well as click-click (Tool.dragCommits). */
  readonly dragCommits = true

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    // A face Offset cannot cut explains itself while hovered (the same line
    // the refused click toasts) — Push/Pull's posture.
    if (this.stage.kind === 'idle' && this._hoverRefusal !== null) return this._hoverRefusal
    if (this.stage.kind === 'idle' && this.retype.isOpen) {
      return t`Type an exact distance to redo the offset you just made — or click another face or profile.`
    }
    return this.stage.kind === 'idle'
      ? t`Click a face or a closed profile to offset its boundary.`
      : t`Drag inward or outward, click to commit — or type an exact distance.`
  }

  /** The hovered face's refusal while idle (see `_updateHoverRefusal`). */
  private _hoverRefusal: string | null = null

  /** Cursor-side guidance (Tool.hoverHint). */
  hoverHint(): string | null {
    return this.stage.kind === 'idle' ? this._hoverRefusal : null
  }

  /** The pointer left the canvas: nothing is hovered (Tool.onPointerLeave). */
  onPointerLeave(): void {
    this._hoverRefusal = null
  }

  /** Why a component member's face refuses Offset — the click's toast and
   *  the hover's line, one wording. */
  private _componentFaceHint(): string {
    return t`Offset can’t cut a component’s member face yet — try a sketch region instead, or explode the instance.`
  }

  private _ineligibleFaceHint(instance: bigint | undefined): string {
    return ineligibleFaceHint(
      t({ context: 'tool', message: 'Offset' }),
      this._editContext.kind !== 'top',
      instance,
      sessionIsOpen(this.wasmScene),
    )
  }

  /**
   * Resolves whether the face under `ray` would refuse a click, deciding
   * exactly as `_pickTarget` does (same pick, same two refusal branches,
   * same order), so the hover never says one thing and the click another.
   */
  private _updateHoverRefusal(ray: Ray): void {
    let next: string | null = null
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (pick !== undefined) {
      try {
        const objectHandle = pick.object()
        const instanceHandle = pick.instance()
        if (instanceHandle !== undefined) next = this._componentFaceHint()
        else if (!this._isEligible(objectHandle, instanceHandle)) next = this._ineligibleFaceHint(instanceHandle)
      } catch {
        next = null
      } finally {
        pick.free()
      }
    }
    this._hoverRefusal = next
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

  /** The offset just committed, redoable at a typed distance until the next
   *  pointer action, Escape, tool switch, or other document mutation (see
   *  `RetypeSpec` and retypeWindow.ts). */
  private readonly retype: RetypeWindow<RetypeSpec>

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
    this.retype = new RetypeWindow(wasmScene)
  }

  // ── Optional Tool interface extensions ────────────────────────────────────

  /** Armed for Escape's purposes while dragging or while a post-commit
   *  distance is being typed (see `toolHasArmedGesture` in tools/types.ts). */
  hasArmedGesture(): boolean {
    return this.capturingInput() || this.typed !== ''
  }

  /** Per-key capture (see Tool.capturesKey): a drag keeps the whole
   *  keyboard as before; the IDLE retype window takes only what a typed
   *  distance needs (`idleRetypeCapturesKey`). */
  capturesKey(key: string): boolean {
    if (this.capturingInput()) return true
    if (!this.retype.isOpen) return false
    return idleRetypeCapturesKey(key, this.typed, isLengthInputKey)
  }

  /** Quietly close the retype window — the host calls this before an
   *  explicit undo/redo/delete (`disarmActivePostCommitWindow`). */
  disarmRetype(): void {
    this.retype.close()
    if (this.stage.kind === 'idle' && this.typed !== '') {
      this.typed = ''
      this.onMeasurementCb('')
    }
  }

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
  /** The Viewport's context-path-aware face eligibility (same predicate the
   *  draw tools and Push/Pull take); `null` falls back to the default. */
  private _faceEligible: FaceEligible | null = null
  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }
  private _isEligible(object: bigint, instance: bigint | undefined): boolean {
    if (this._faceEligible !== null) return this._faceEligible(object, instance)
    return defaultFaceEligible(this.wasmScene, this._activeContext, object, instance)
  }

  private get _activeContext(): bigint | null {
    return this._editContext.kind === 'object' ? this._editContext.id : null
  }

  /**
   * The entered component INSTANCE id, or null. `pick_sketch_region` only
   * walks world-tree sketches, so Path B of `_pickTarget` calls
   * `pick_sketch_region_in_instance` instead whenever this is non-null —
   * exactly like `PushPullTool`'s Path B — and the sketch's context carries
   * the instance so the draft opens inside it.
   */
  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    this.lastSnap = snap
    if (this.stage.kind !== 'dragging') {
      this._updateHoverRefusal(ray)
      return
    }

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
    // Any pointer action ends the retype window — the next offset begins.
    this.disarmRetype()
    this._hoverRefusal = null
    if (this.stage.kind === 'idle') {
      this._pickTarget(ray)
      return
    }

    // Second click: commit at the click's own distance. A click while the
    // cursor is off the plane (no live distance) commits nothing — the drag
    // stays open until the cursor is back over the plane or Esc cancels.
    const fresh = this._dragDistance(snap, ray)
    const final = fresh ?? this.stage.distance
    if (final === null) return
    // A distance that follows the cursor (not borrowed from a hard snap)
    // commits the DISPLAYED value through the typed path — sign rules
    // included — exactly as if typed. A hard-snap distance stays exact.
    const shown = displayedLength(Math.abs(final))
    if (fresh !== null && !(snap !== null && HARD_SNAP_KINDS.has(snap.kind)) && shown >= 1e-6) {
      this.stage = { ...this.stage, distance: fresh, lastValidDistance: fresh }
      this._commitFromTyped(shown)
      return
    }
    this._commit(final)
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }
    // Idle retype window (see `RetypeSpec`).
    if (this.stage.kind === 'idle' && this.retype.isOpen && this.capturesKey(ev.key)) {
      if (ev.key === 'Enter') {
        const meters = parseLengthToMeters(this.typed)
        this.typed = ''
        this.onMeasurementCb('')
        if (meters !== null) this._retypeDistance(meters)
        return
      }
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this.typed === '' ? '' : typedReadout(this.typed))
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
    this.retype.close()
    this.stage = { kind: 'idle' }
    this.typed = ''
    this._clearPreview()
    this.onMeasurementCb('')
    this.lastSnap = null
    this._hoverRefusal = null
  }

  // ── Stage 1: pick ──────────────────────────────────────────────────────────

  private _pickTarget(ray: Ray): void {
    const scene = this.wasmScene as unknown as PlaneUnitsScene
    // Path A: nearest object face (scoped to the editing context, like
    // Push/Pull — inside a context only the entered object is editable). The
    // face names the plane; the plane's listing names the region.
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (pick !== undefined) {
      try {
        const objectHandle = pick.object()
        const instanceHandle = pick.instance()
        // A component member's face is refused with a hint rather than
        // silently ignored — ALWAYS toast, regardless of context: the far
        // more common case is a top-level click on an ordinary placed
        // instance, which must not swallow the click with no feedback
        // (PushPullTool's `_ineligibleFaceHint` is the precedent).
        if (instanceHandle !== undefined) {
          this.onToast(this._componentFaceHint(), EDIT_CONTEXT_HINT_CODE)
          return
        }
        // The draw tools' and Push/Pull's eligibility policy: a face inside a
        // group (not entered) or outside the editing scope refuses with the
        // same hint, never a silent no-op.
        if (!this._isEligible(objectHandle, instanceHandle)) {
          this.onToast(this._ineligibleFaceHint(instanceHandle), EDIT_CONTEXT_HINT_CODE)
          return
        }
        if (this._activeContext === null || objectHandle === this._activeContext) {
          const ctx = ctxOfFace(this.wasmScene, objectHandle, pick.face(), null)
          if (ctx !== null) {
            const hit = regionUnderRay(scene, ctx, ray, EDGE_PICK_FRACTION * Math.max(pick.depth(), 0))
            if (hit !== null) this._beginRegionDrag(hit)
          }
          return
        }
      } finally {
        pick.free()
      }
    }

    // Path B: no object face — a free sketch's region (top-level OR
    // instance-context act — an instance context has its own def-owned
    // regions too). `pick_sketch_region` only walks world-tree sketches, so an
    // instance context calls the `_in_instance` sibling, exactly like
    // `PushPullTool`'s Path B. The sketch names the plane (the ground for a
    // ground sketch); the listing names the region.
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
      // missed. Fall back to the edge pick (which has the proper tolerance)
      // and resolve the edge to the region whose boundary carries it. World
      // sketches only: `pick_sketch_edge` has no `_in_instance` sibling.
      const viaEdge = this._pickRegionViaEdge(ray)
      if (viaEdge !== null) this._beginRegionDrag(viaEdge)
      return
    }
    try {
      const sketch = regionPick.sketch()
      const ctx = ctxOfSketch(this.wasmScene, sketch, activeInstance)
      if (ctx === null) return // stale handle — stay idle
      const hit =
        regionOfFreeProfile(scene, ctx, sketch, regionPick.region()) ??
        regionUnderRay(scene, ctx, ray, 0)
      if (hit !== null) this._beginRegionDrag(hit)
    } finally {
      regionPick.free()
    }
  }

  /** Resolve a click that landed on a free sketch EDGE to the region whose
   *  boundary carries it. Null when nothing usable is under the cursor
   *  (open-line edge, stale handles, no sketch at all). */
  private _pickRegionViaEdge(ray: Ray): PickedRegion | null {
    const edgePick = this.wasmScene.pick_sketch_edge(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (edgePick === undefined) return null
    try {
      const sketch = edgePick.sketch()
      const ends = this.wasmScene.sketch_edge_endpoints(sketch, edgePick.edge())
      if (ends === undefined || ends.length < 6) return null
      const ctx = ctxOfSketch(this.wasmScene, sketch, null)
      if (ctx === null) return null
      return regionOfSegment(
        this.wasmScene as unknown as PlaneUnitsScene,
        ctx,
        [ends[0], ends[1], ends[2]],
        [ends[3], ends[4], ends[5]],
      )
    } finally {
      edgePick.free()
    }
  }

  /** Start the drag on a picked region: the drag plane is the region's own
   *  plane (the context's), anchored at its first outer vertex. */
  private _beginRegionDrag(hit: PickedRegion): void {
    const { outer } = hit.region
    if (outer.length < 3) return
    const boundary = new Float32Array(outer.length * 3)
    outer.forEach((p, i) => boundary.set(p, i * 3))
    this._beginDrag(
      { ctx: hit.ctx, regionKey: hit.region.key, onFace: hit.region.over !== null },
      boundary,
      hit,
      hit.point,
      hit.normal,
    )
  }

  private _beginDrag(
    target: OffsetTarget,
    boundary: Float32Array,
    region: PickedRegion,
    planePoint: V3,
    planeNormal: V3,
  ): void {
    this.typed = ''
    this.stage = {
      kind: 'dragging',
      target,
      boundary,
      region,
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
    const { region } = this.stage

    // The ghost is the kernel's own offset of the region (exact: arcs keep
    // their circle); an impossible distance (collapse, spiked corner) draws
    // nothing — the vanished preview IS the feedback, and a commit would
    // surface the kernel's typed error.
    let loops: number[][]
    try {
      loops = decodeOffsetLoops(
        this.wasmScene.offset_region_preview(region.ctx, keyToId(region.region.key), distance),
      )
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
          : target.onFace
            ? -1
            : 1
    this._commit(Math.abs(meters) * sign)
  }

  private _commit(distance: number): void {
    if (this.stage.kind !== 'dragging') return
    const { target } = this.stage

    if (Math.abs(distance) < 1e-6) {
      this.onToast(t`Move more before committing the offset`)
      return
    }

    this.stage = { kind: 'idle' }
    this.typed = ''
    this._clearPreview()
    this.onMeasurementCb('')

    const genBefore = this.wasmScene.history_generation()
    if (this._commitTarget(target, distance)) {
      this.retype.armFrom({ target, distance }, genBefore)
    }
  }

  /**
   * Redo the offset just committed at |`d`| — the committed direction for a
   * positive value, the opposite for a negative one — through the shared
   * window (undo the commit, re-commit, roll back on refusal).
   */
  private _retypeDistance(d: number): void {
    const spec = this.retype.spec
    if (spec === null) return
    if (Math.abs(d) < 1e-6) {
      this.onToast(t`Move more before committing the offset`)
      return
    }
    const signed = (spec.distance < 0 ? -1 : 1) * d
    const outcome = this.retype.apply(
      (hot) => this._commitTarget(hot.target, signed),
      (hot) => this._commitTarget(hot.target, hot.distance),
      (hot) => ({ ...hot, distance: signed }),
    )
    if (outcome === 'stale') this.onToast(retypeStaleMessage('offset'))
  }

  /** The kernel commit for `target` at `distance` — ONE draft on the
   *  region's context. True when accepted (false = refused, toasted). */
  private _commitTarget(target: OffsetTarget, distance: number): boolean {
    try {
      const { report } = runDraft(this.wasmScene, { kind: 'ctx', ctx: target.ctx }, t({ context: 'tool', message: 'Offset' }), (d) => {
        this.wasmScene.draft_offset_region(d, keyToId(target.regionKey), distance)
      })
      refreshAfterDraft(report, {
        onObject: (o) => this.onFaceImprint(o),
        onSketch: () => this.onCommit(target),
      })
      return true
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
      return false
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
