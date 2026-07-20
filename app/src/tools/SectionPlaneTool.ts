/**
 * SectionPlaneTool — Tools ▸ Section Plane: a NON-DESTRUCTIVE clipping plane
 * for looking inside a model (wall thickness, clearance, voids). Distinct
 * from `SliceTool`, which destructively splits a solid into two Objects —
 * Section never touches the document; it only drives the render-time clip
 * owned by `sectionManager.ts`/`SceneRenderer`. DESIGN §1/§5 hold the full
 * behavioral contract this implements.
 *
 * Three gestures, all through the plain `Tool` interface (`onPointerMove` /
 * `onPointerDown` / `onKey` — there is no pointer-up in this interface, so
 * every gesture here is click-move-click, the same idiom `PushPullTool` and
 * `SliceTool` already use):
 *
 *   1. **Place** (idle stage, click NOT on the current widget): resolves a
 *      plane from the hovered face (`face_plane`, coincident with + normal
 *      to it) or, off any face, an axis-aligned horizontal plane at the
 *      ground (Z=0, DESIGN §1) under the cursor. Always REPLACES the
 *      current section (one at a time — DESIGN §1) via `onPlace`. The tool
 *      STAYS ACTIVE after placing (it does NOT spring back to Select) so the
 *      user can immediately sweep, toggle, or delete the section — the
 *      natural inspect-tool flow, and what lets Delete target the section
 *      rather than the document selection (a spring-back to Select would
 *      leave Delete deleting the selected object, since the App-level Delete
 *      guard keys off THIS tool being active). A deliberate deviation from
 *      DESIGN §1's "springs back to Select", driven by the review's
 *      Delete-must-not-destroy-the-object check.
 *   2. **Offset-sweep the widget** (idle stage, click ON the current
 *      widget's rectangle — `_hitWidget`): arms an `offsetting` stage.
 *      Every further pointer move projects the ray onto the plane's own
 *      normal axis (`projectRayOntoAxis`, the same closest-point-on-a-line
 *      math `PushPullTool` uses for its extrude drag) to get a live delta
 *      from the grab point, previewed via `onOffsetPreview` (cheap,
 *      renderer-side, no material rebuild — see `SceneRenderer.
 *      updateSectionPlaneOffset`). The gesture resolves on the SECOND
 *      click or Enter (typed exact offset, reusing the VCB machinery). A
 *      SECOND click that still lands on the (moved) widget commits the sweep
 *      past `OFFSET_DRAG_THRESHOLD_M` (`onOffsetCommit`) or, within
 *      threshold, toggles active (`onToggle`); a second click that MISSES
 *      the widget abandons the arm and re-places on whatever it hit instead
 *      (so "click widget, then click a face" re-places rather than jumping
 *      the offset). This tool does NOT spring back to Select after a
 *      sweep/toggle — sweeping is "the primary inspection gesture" (DESIGN
 *      §1), meant to be repeated.
 *
 *      Note the click-move-click reality: a single physical tap on the
 *      widget only ARMS the stage; it does not resolve on its own (the
 *      `Tool` interface has no pointer-up). The dependable one-action toggle
 *      is therefore the **Tools ▸ Toggle Section Active** command
 *      (`toggleSectionActive`), which every doc/status-hint points users to
 *      rather than promising a single-click toggle this gesture can't
 *      deliver; the within-threshold toggle path here is a convenience for
 *      the "grab, decide not to move, release" case.
 *   3. **Delete** (idle stage, Delete/Backspace, a section exists): removes
 *      the section via `onDelete` — "the model returns to whole." The tool
 *      captures Delete/Backspace (via `capturesKey`) ONLY while a section
 *      exists, so the App-level Delete handler backs off and never ALSO
 *      deletes the document selection. Esc during an offset-drag reverts to
 *      the committed plane instead (`onCancelOffset`) rather than deleting.
 *
 * The section entity is deliberately NOT folded into the app's generic
 * NodeRef/Outliner selection system (DESIGN §8 scopes out an Outliner entry
 * and full Move-tool integration for v0.3.0) — this tool owns its own
 * pick/drag/delete lifecycle end to end, self-contained, the same way
 * `SceneRenderer`'s construction-guide picking (`pickGuide` in Viewport.tsx)
 * is a bespoke, lighter-weight sibling of full node selection rather than a
 * NodeRef itself.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { facePlaneBasis, projectRayOntoAxis, rayPlaneIntersect, type V3 } from '../viewport/geoHelpers'
import { createSectionPlane, offsetSectionPlane, type SectionPlane } from '../viewport/sectionManager'
import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'

export type OnSectionPlace = (origin: V3, normal: V3) => void
export type OnSectionOffsetPreview = (plane: SectionPlane) => void
export type OnSectionOffsetCommit = (plane: SectionPlane) => void
export type OnSectionToggle = () => void
export type OnSectionDelete = () => void
export type OnSectionCancelOffset = () => void
export type GetCurrentSectionPlane = () => SectionPlane | null
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Net world-space movement (meters) along the plane's normal that turns a
 * press-on-the-widget into a genuine sweep rather than a plain click
 * (toggle) — the world-space counterpart of `dragMove.ts`'s pixel
 * `DRAG_MOVE_THRESHOLD_PX` (that threshold is unavailable here: the `Tool`
 * interface hands gestures a `Ray`, not screen pixels — see the module doc).
 * 1 mm comfortably exceeds float/ray noise on a static click while staying
 * far below any deliberate sweep. */
export const OFFSET_DRAG_THRESHOLD_M = 0.001

/** Preview/placement color — matches `SceneRenderer`'s committed widget
 * color (kept as a literal in both places rather than a cross-module import;
 * see that file's `SECTION_WIDGET_COLOR`). Distinct from Slice's purple
 * preview so the two tools never look like the same feature. */
const PREVIEW_COLOR = 0x00bcd4
const PREVIEW_FILL_OPACITY = 0.22
/** Screen-constant scale for the PLACEMENT preview quad (not the committed
 * widget, which SceneRenderer sizes to the real scene bounds) — mirrors
 * SliceTool's PLANE_SCREEN_K so a hover preview reads at a steady size. */
const PREVIEW_SCREEN_K = 0.06
const PREVIEW_BASE_HALF = 1

function normalizeOrZ(v: V3): V3 {
  const len = Math.hypot(v[0], v[1], v[2])
  return len < 1e-9 ? [0, 0, 1] : [v[0] / len, v[1] / len, v[2] / len]
}

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'offsetting'
      /** The plane as it was when the widget was grabbed — Esc reverts to
       * exactly this, and every live delta is measured from it. */
      basePlane: SectionPlane
      /** Axis-projected distance of the GRAB point itself (not necessarily
       * 0 — the click may land anywhere on the widget), so `delta` below is
       * relative to where the user actually grabbed it. */
      baseDistance: number
      /** Latest net signed offset from the grab point, meters. */
      delta: number
    }

export class SectionPlaneTool implements Tool {
  readonly name = 'Section Plane'

  /** Live status-bar guidance (see Tool.statusHint). */
  statusHint(): string {
    if (this.stage.kind === 'offsetting') {
      return 'Move to sweep the cut, click to set it — or type an exact distance. Esc reverts.'
    }
    return this.getCurrentPlane() !== null
      ? 'Click the widget then move to sweep the cut; click a face to re-place. Delete removes it; toggle it off from Tools ▸ Toggle Section Active.'
      : 'Click a face to section it there, or click the ground for a horizontal cut.'
  }

  /** Last snap seen on hover (idle stage only — offsetting has its own
   * axis-projected math and does not consult snap). */
  lastSnap: Snap | null = null

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private previewPlane: THREE.Group | null = null
  private wasmScene: WasmScene
  private getCurrentPlane: GetCurrentSectionPlane
  /** Half-extent (meters) of the CURRENT widget rectangle, snapshotted at
   * tool construction — used only for `_hitWidget`'s bounds check. Matches
   * `SceneRenderer.sectionWidgetHalfExtent()` at the moment the tool was
   * activated; see that method's doc comment for why re-querying live on
   * every pointer move is deliberately not done. */
  private widgetHalfExtent: number
  private onPlace: OnSectionPlace
  private onOffsetPreview: OnSectionOffsetPreview
  private onOffsetCommit: OnSectionOffsetCommit
  private onToggle: OnSectionToggle
  private onDelete: OnSectionDelete
  private onCancelOffset: OnSectionCancelOffset
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** VCB buffer — raw string being typed by the user (offset along the normal). */
  private typed: string = ''

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    getCurrentPlane: GetCurrentSectionPlane,
    widgetHalfExtent: number,
    onPlace: OnSectionPlace,
    onOffsetPreview: OnSectionOffsetPreview,
    onOffsetCommit: OnSectionOffsetCommit,
    onToggle: OnSectionToggle,
    onDelete: OnSectionDelete,
    onCancelOffset: OnSectionCancelOffset,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.getCurrentPlane = getCurrentPlane
    this.widgetHalfExtent = widgetHalfExtent
    this.onPlace = onPlace
    this.onOffsetPreview = onOffsetPreview
    this.onOffsetCommit = onOffsetCommit
    this.onToggle = onToggle
    this.onDelete = onDelete
    this.onCancelOffset = onCancelOffset
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  // ── Optional Tool interface extensions ─────────────────────────────────

  /** True while the offset-drag's typed VCB is live — the coarse verdict for
   * callers that don't pass a key. Per-key routing goes through `capturesKey`
   * below (the App-level Delete handler and the Viewport's keydown routing
   * both prefer it when present). */
  capturingInput(): boolean {
    return this.stage.kind === 'offsetting'
  }

  /**
   * Per-key refinement of `capturingInput` (see Tool.capturesKey). Two cases:
   *
   *  - **Offsetting**: capture the whole keyboard, like MoveTool's live
   *    two-click stage — the length VCB legitimately eats letters (unit
   *    suffixes) and Space ("5' 3"), and no bare keystroke should leak out
   *    mid-drag.
   *  - **Idle**: capture Delete/Backspace ONLY when a section actually exists
   *    to remove. This is the fix for the destructive double-delete: the
   *    App-level Delete/Backspace handler backs off exactly when this returns
   *    true, so with a section placed, Delete removes ONLY the section and
   *    never also runs a real kernel delete of the document selection. With
   *    NO section placed, Delete falls through to its normal meaning (delete
   *    the selection) — this tool has nothing of its own to delete.
   *
   * Everything else (tool-switch letters, etc.) falls through to its global
   * meaning while idle.
   */
  capturesKey(key: string): boolean {
    if (this.stage.kind === 'offsetting') return true
    if (key === 'Delete' || key === 'Backspace') return this.getCurrentPlane() !== null
    return false
  }

  // ── Tool interface ──────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'offsetting') {
      const { basePlane, baseDistance } = this.stage
      const cursorDistance = projectRayOntoAxis(ray.origin, ray.direction, basePlane.origin, basePlane.normal)
      const delta = cursorDistance - baseDistance
      this.stage = { ...this.stage, delta }
      this.onOffsetPreview(offsetSectionPlane(basePlane, delta))
      this._reportOffset(delta)
      return
    }

    this.lastSnap = snap
    if (snap === null || this._hitWidget(ray) !== null) {
      // Hovering the widget (or off the model entirely) previews nothing —
      // a widget hover is the more specific gesture (§ module doc point 2).
      this._clearPreview()
      return
    }
    this._buildPreviewPlane(this._resolveOrigin(snap), this._resolveNormal(snap))
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'offsetting') {
      // The second click resolves the drag ONLY when it still lands on the
      // widget (the widget slid to `basePlane` offset by the current delta,
      // so test THAT previewed rectangle, not the committed one). A click
      // that misses the widget — e.g. on a distant face to re-place — must
      // not produce a surprise offset jump: abandon the arm (reverting the
      // live preview) and fall through to a fresh placement below.
      const { basePlane, delta } = this.stage
      if (this._rayHitsWidgetPlane(ray, offsetSectionPlane(basePlane, delta))) {
        this._commitOffsetOrToggle(ray)
        return
      }
      this.stage = { kind: 'idle' }
      this.typed = ''
      this.onMeasurementCb('')
      this.onCancelOffset()
      // fall through to placement
    }

    const hit = this._hitWidget(ray)
    if (hit !== null) {
      this.typed = ''
      this.stage = { kind: 'offsetting', basePlane: hit.plane, baseDistance: hit.distanceAlongNormal, delta: 0 }
      this._clearPreview()
      return
    }

    if (snap === null) return
    const origin = this._resolveOrigin(snap)
    const normal = this._resolveNormal(snap)
    this._clearPreview()
    this.onPlace(origin, normal)
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    if (this.stage.kind === 'idle') {
      if ((ev.key === 'Delete' || ev.key === 'Backspace') && this.getCurrentPlane() !== null) {
        this.onDelete()
      }
      return
    }

    // 'offsetting' — numeric VCB, same grammar as PushPull/Slice/Move.
    if (ev.key === 'Enter') {
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) this._commitOffsetFromTyped(meters)
      return
    }
    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this._typedReadout())
    }
  }

  /** The typed-buffer readout, suffixed for metric formats. */
  private _typedReadout(): string {
    return typedReadout(this.typed)
  }

  cancel(): void {
    // A cancel mid-drag (Esc, or the Viewport switching tools out from under
    // an in-progress sweep) must revert the LIVE preview back to the
    // committed plane — otherwise the widget is left rendered at an
    // uncommitted offset while sectionManager still holds the old one.
    if (this.stage.kind === 'offsetting') {
      this.onCancelOffset()
    }
    this.stage = { kind: 'idle' }
    this.typed = ''
    this._clearPreview()
    this.lastSnap = null
    this.onMeasurementCb('')
  }

  // ── Private helpers ─────────────────────────────────────────────────────

  private _reportOffset(delta: number): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    this.onMeasurementCb(formatLength(delta))
  }

  private _commitOffsetFromTyped(delta: number): void {
    if (this.stage.kind !== 'offsetting') return
    const { basePlane } = this.stage
    this.stage = { kind: 'idle' }
    this.typed = ''
    this.onMeasurementCb('')
    this.onOffsetCommit(offsetSectionPlane(basePlane, delta))
  }

  /**
   * Resolve the SECOND click of the offset-drag. Recomputes the delta fresh
   * from THIS click's own ray (mirroring `PushPullTool._commit`'s re-derive
   * at commit) rather than trusting the last hover move's stored value, so a
   * click that lands a frame after the last move still reflects exactly
   * where the pointer went down. Past the world-space threshold it's a
   * genuine sweep; under it, it's a plain click on the widget — toggle
   * active (DESIGN §1's "clicking the widget toggle[s] the section").
   */
  private _commitOffsetOrToggle(ray: Ray): void {
    if (this.stage.kind !== 'offsetting') return
    const { basePlane, baseDistance } = this.stage
    const cursorDistance = projectRayOntoAxis(ray.origin, ray.direction, basePlane.origin, basePlane.normal)
    const delta = cursorDistance - baseDistance
    this.stage = { kind: 'idle' }
    this.typed = ''
    this.onMeasurementCb('')
    if (Math.abs(delta) < OFFSET_DRAG_THRESHOLD_M) {
      this.onToggle()
      return
    }
    this.onOffsetCommit(offsetSectionPlane(basePlane, delta))
  }

  /**
   * Whether `ray` hits the rectangle of `plane`'s widget: a ray/plane
   * intersection followed by an in-plane bounds check against
   * `widgetHalfExtent` (the same half-extent `SceneRenderer` sizes the
   * rendered rectangle to — see that method's doc comment for why this is a
   * construction-time snapshot rather than a live query). False when the ray
   * is parallel to the plane or the hit falls outside the rectangle.
   */
  private _rayHitsWidgetPlane(ray: Ray, plane: SectionPlane): boolean {
    const hitPoint = rayPlaneIntersect(ray.origin, ray.direction, plane.origin, plane.normal)
    if (hitPoint === null) return false
    const basis = facePlaneBasis(plane.normal)
    if (basis === null) return false
    const dx = hitPoint[0] - plane.origin[0]
    const dy = hitPoint[1] - plane.origin[1]
    const dz = hitPoint[2] - plane.origin[2]
    const pu = dx * basis.u[0] + dy * basis.u[1] + dz * basis.u[2]
    const pv = dx * basis.v[0] + dy * basis.v[1] + dz * basis.v[2]
    return Math.abs(pu) <= this.widgetHalfExtent && Math.abs(pv) <= this.widgetHalfExtent
  }

  /**
   * Whether `ray` hits the CURRENT (committed) widget — the idle-stage
   * arm-the-drag test. Returns the hit plane plus the grab point's
   * axis-projected distance (the `offsetting` stage's baseline), or null when
   * there is no section or the ray misses the rectangle.
   */
  private _hitWidget(ray: Ray): { plane: SectionPlane; distanceAlongNormal: number } | null {
    const plane = this.getCurrentPlane()
    if (plane === null || !this._rayHitsWidgetPlane(ray, plane)) return null
    return {
      plane,
      distanceAlongNormal: projectRayOntoAxis(ray.origin, ray.direction, plane.origin, plane.normal),
    }
  }

  /** Resolve the placement normal for a hover/click snap: the hovered live
   * world-Object face's plane normal if resolvable, else a horizontal (+Z)
   * plane. Both the genuine ground click and every other non-face snap use
   * +Z; they differ only in the origin's height (see `_resolveOrigin`). A
   * face-aligned normal for an arbitrary edge/vertex snap is a v0.3.0
   * follow-up. */
  private _resolveNormal(snap: Snap): V3 {
    if (snap.elementKind === 'face' && snap.object !== undefined && snap.element !== undefined) {
      try {
        const plane = this.wasmScene.face_plane(snap.object, snap.element)
        return normalizeOrZ([plane[3], plane[4], plane[5]])
      } catch {
        // Not a live world-Object face (e.g. instanced geometry) — fall through.
      }
    }
    return [0, 0, 1]
  }

  /** Resolve the placement origin from a snap:
   *  - **face** → the exact snapped point on the face (coincident, DESIGN §1);
   *  - **genuine empty ground** (`snap.kind === 'ground'`) → the cursor's
   *    ground point AT Z=0, so the horizontal plane sits at the ground itself;
   *  - **any other snap** (vertex / edge / sketch, etc.) → the snap's REAL
   *    position, Z included. Collapsing Z to 0 here was a bug: inference
   *    prefers vertex/edge snaps near corners, so clicking near a raised
   *    face's edge would silently drop the section to the floor instead of
   *    cutting at that height. */
  private _resolveOrigin(snap: Snap): V3 {
    if (snap.elementKind === 'face') return [snap.x, snap.y, snap.z]
    if (snap.kind === 'ground') return [snap.x, snap.y, 0]
    return [snap.x, snap.y, snap.z]
  }

  // ── Placement preview (idle stage only) ─────────────────────────────────

  private _buildPreviewPlane(center: V3, normal: V3): void {
    this._clearPreview()

    const basis = facePlaneBasis(normal)
    if (basis === null) return
    const { u, v } = basis

    const corners: [number, number][] = [
      [-PREVIEW_BASE_HALF, -PREVIEW_BASE_HALF],
      [PREVIEW_BASE_HALF, -PREVIEW_BASE_HALF],
      [PREVIEW_BASE_HALF, PREVIEW_BASE_HALF],
      [-PREVIEW_BASE_HALF, PREVIEW_BASE_HALF],
    ]
    const toLocal = ([cu, cv]: [number, number]): V3 => [
      u[0] * cu + v[0] * cv,
      u[1] * cu + v[1] * cv,
      u[2] * cu + v[2] * cv,
    ]
    const worldCorners = corners.map(toLocal)

    const tri1 = [worldCorners[0], worldCorners[1], worldCorners[2]]
    const tri2 = [worldCorners[0], worldCorners[2], worldCorners[3]]
    const fillPts = new Float32Array(18)
    let i = 0
    for (const p of [...tri1, ...tri2]) {
      fillPts[i++] = p[0]
      fillPts[i++] = p[1]
      fillPts[i++] = p[2]
    }
    const fillGeo = new THREE.BufferGeometry()
    fillGeo.setAttribute('position', new THREE.BufferAttribute(fillPts, 3))
    const fillMat = new THREE.MeshBasicMaterial({
      color: PREVIEW_COLOR,
      transparent: true,
      opacity: PREVIEW_FILL_OPACITY,
      side: THREE.DoubleSide,
      depthTest: false,
    })
    const fillMesh = new THREE.Mesh(fillGeo, fillMat)

    const outlinePts = new Float32Array(12)
    i = 0
    for (const p of worldCorners) {
      outlinePts[i++] = p[0]
      outlinePts[i++] = p[1]
      outlinePts[i++] = p[2]
    }
    const outlineGeo = new THREE.BufferGeometry()
    outlineGeo.setAttribute('position', new THREE.BufferAttribute(outlinePts, 3))
    const outlineMat = new THREE.LineBasicMaterial({ color: PREVIEW_COLOR, depthTest: false })
    const outline = new THREE.LineLoop(outlineGeo, outlineMat)

    const group = new THREE.Group()
    group.add(fillMesh)
    group.add(outline)
    group.position.set(center[0], center[1], center[2])
    group.scale.setScalar(PREVIEW_SCREEN_K * 4)
    this.preview.add(group)
    this.previewPlane = group
  }

  /** Keep the preview quad a constant on-screen size regardless of zoom —
   * called once per frame by the Viewport render loop (feature-detected,
   * matching ProtractorTool/SliceTool's own `updateDiskScale`). */
  updateDiskScale(camera: THREE.Camera): void {
    if (this.previewPlane === null) return
    const dist = camera.position.distanceTo(this.previewPlane.position)
    this.previewPlane.scale.setScalar(PREVIEW_SCREEN_K * dist)
  }

  private _clearPreview(): void {
    if (this.previewPlane === null) return
    for (const child of this.previewPlane.children) {
      if (child instanceof THREE.Mesh || child instanceof THREE.LineLoop) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    }
    this.preview.remove(this.previewPlane)
    this.previewPlane = null
  }
}
