/**
 * RotateTool — SketchUp-style rotation about an arbitrary axis, driven by a
 * live "protractor" widget (a screen-constant disk that shows, and lets you
 * lock, the rotation axis before you commit to it).
 *
 * The protractor (a 360° ring lying in the rotation plane, centered on the
 * cursor) is visible the moment the tool is active. Its NORMAL is the rotation
 * axis; the ring is colored by that axis (X=red / Y=green / Z=blue, blue by
 * default because the ground plane's normal is +Z), or neutral purple when the
 * axis is off every world axis. This makes the axis you're about to rotate
 * around visible up front — the single biggest source of "the rotation went
 * somewhere I didn't expect", especially for cylinders, whose curved side
 * offers no face to infer an axis from.
 *
 * Axis inference & locking (works from the idle/hover phase onward, so you can
 * settle the axis BEFORE placing the center — the SketchUp order):
 *   - Hover a face  → axis = that face's normal.
 *   - Hover an edge → axis = the edge's direction (spin about the edge).
 *   - Otherwise     → axis = world +Z (ground-plane spin).
 *   - Shift toggles a lock on the current axis (the ring renders at full
 *     opacity with a short normal tick so the lock is obvious). Toggle, not
 *     hold — matches Slice/Protractor.
 *   - Arrow keys force-lock a world axis: → X, ← Y, ↑ Z; ↓ clears the lock
 *     and returns to inference. Locking with an arrow is how you tip a
 *     cylinder onto its side (see the status hint).
 *
 * Gesture (three-click), once the axis is settled:
 *   1. Click  : set the pivot (center of rotation). The axis is captured here.
 *   2. Click  : set the start-reference point (the 0° arm).
 *   3. Move   : sweep — a ghost of the selection rotates live; the angle snaps
 *               to 15° increments unless a value is typed. The protractor draws
 *               a dim baseline arm (0°) and a colored swept arm at the angle.
 *   4. Click  : commit the rotation by the live delta about the pivot (a single
 *               node → its per-kind transform; a multi-selection → one
 *               transform_selection call, one undo step).
 *   5. Esc    : cancel the gesture (and clear any axis lock).
 *
 * Numeric VCB (while sweeping): type digits / . / - to build an angle buffer;
 * Enter commits that exact number of DEGREES (unitless) about the effective
 * axis.
 *
 * The rotation plane is the plane through the pivot whose normal is the
 * effective axis. Because that plane often has no snappable geometry (a
 * vertical plane floating in space), the live sweep uses ray/plane
 * intersection rather than the resolved snap point.
 *
 * If nothing is selected, the first click auto-selects whatever is under the
 * cursor (via the Viewport-injected selection acquirer) and starts the
 * rotation on it in the same gesture; only a click over empty space shows a
 * hint toast and stays idle.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  rotateAboutPivotAxis,
  rotationAxisAffine,
  signedAngleAboutAxis,
  snapAngleDeg,
  affineToFloat64,
  projectOntoPlane,
  planeBasis,
  normalize3,
} from './transformMath'
import { rayPlaneIntersect } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'
import { commitSelectionTransform, buildSelectionPreview } from './transformSelection'
import { arrowToAxis, editNumericBuffer, parseDistance } from './moveInput'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import type { NodeRef } from '../panels/treeModel'

export type OnRotateCommit = (nodes: NodeRef[]) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

type Vec3 = [number, number, number]

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'pivot'
      nodes: NodeRef[]
      pivot: Vec3
      /** Rotation axis captured at the pivot click (unit). Overridable by a
       * later Shift/arrow lock via `lockedNormal`. */
      axis: Vec3
    }
  | {
      kind: 'ref'
      nodes: NodeRef[]
      pivot: Vec3
      axis: Vec3
      /** The 3D reference point (2nd click); the baseline (0°) arm is this
       * point projected into the rotation plane. */
      refPoint: Vec3
      previewMesh: THREE.Object3D | null
      /** Last computed delta (radians, snapped) — held steady when the cursor
       * ray is parallel to the rotation plane. */
      lastDelta: number
    }

const SNAP_DEG = 15
const WORLD_AXIS: Record<0 | 1 | 2, Vec3> = { 0: [1, 0, 0], 1: [0, 1, 0], 2: [0, 0, 1] }
const AXIS_LABEL: Record<0 | 1 | 2, string> = { 0: 'X', 1: 'Y', 2: 'Z' }
/** Default axis: world +Z, the ground plane's normal (a blue protractor). */
const WORLD_UP: Vec3 = [0, 0, 1]
/** Neutral (off-axis) ring color — matches Slice/Protractor's neutral plane. */
const NEUTRAL_PREVIEW_COLOR = 0x9933cc
/** Dim color for the baseline (0°) arm, so the colored swept arm reads against it. */
const BASELINE_ARM_COLOR = 0x888888
/** Axis-color tolerance: within ~2° of a world axis, expressed as cos(θ). */
const AXIS_SNAP_TOL_DOT = Math.cos((2 * Math.PI) / 180)
/** Local radius the ring/arm geometry is built at; the group is scaled to keep
 * it a constant on-screen size (see DISK_SCREEN_K / updateDiskScale). */
const DISK_UNIT_RADIUS = 1.0
/** Sample count for the protractor ring. */
const DISK_SEGMENTS = 64
/** Length of the locked-axis normal tick, as a fraction of the unit radius. */
const DISK_TICK_LENGTH = DISK_UNIT_RADIUS * 0.5
/** Screen-constant scale factor: worldRadius = DISK_SCREEN_K * cameraDistance.
 * ~0.06 sizes the protractor to roughly the Slicer's section-plane gizmo. */
const DISK_SCREEN_K = 0.06

interface Spoke {
  dir: Vec3
  color: number
}

export class RotateTool implements Tool {
  readonly name = 'Rotate'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    switch (this.stage.kind) {
      case 'pivot':
        return 'Click a start point for the angle. Shift or → / ← / ↑ lock the rotation axis.'
      case 'ref':
        return 'Move to set the angle (snaps to 15°), or type exact degrees, then click. Shift or → / ← / ↑ lock the axis.'
      default:
        return this.selection.length === 0
          ? 'Click the object you want to rotate.'
          : 'Click to set the center of rotation. The protractor tilts to the face or edge under the cursor — Shift locks that axis, or press → / ← / ↑ to lock X / Y / Z (needed to tip a cylinder onto its side).'
    }
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnRotateCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement
  private selection: NodeRef[] = []
  private objectsGroup: THREE.Group | null = null
  private instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null

  /** Auto-select fallback, injected by the Viewport (see MoveTool's). */
  private acquireSelection: ((ray: Ray) => NodeRef[] | null) | null = null
  setSelectionAcquirer(acquire: ((ray: Ray) => NodeRef[] | null) | null): void {
    this.acquireSelection = acquire
  }
  /** Keep the cached targets in step with the app selection (Tool.
   * setSelection; see MoveTool) — the next gesture starts from live
   * handles after an undo/redo prune. */
  setSelection(nodes: NodeRef[]): void {
    this.selection = nodes
  }

  /** Axis locked by Shift/arrow (unit). Overrides inference; null = infer. */
  private lockedNormal: Vec3 | null = null
  /** Axis inferred from the hovered face/edge (idle only). Null before the
   * first move. */
  private candidateNormal: Vec3 | null = null
  /** Last snapped cursor point, so the idle protractor can be re-centered after
   * a lock-key change that didn't come with a pointer move. */
  private lastSnapPoint: Vec3 | null = null
  /** The protractor widget (ring + optional lock tick + optional arms). */
  private previewDisk: THREE.Group | null = null
  /** VCB buffer — raw string being typed by the user. */
  private typed: string = ''

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selection: NodeRef[],
    onCommit: OnRotateCommit,
    onToast: OnToast,
    instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.objectsGroup = objectsGroup
    this.selection = selection
    this.onCommit = onCommit
    this.onToast = onToast
    this.instanceGroupGetter = instanceGroupGetter
    this.onMeasurementCb = onMeasurement
  }

  // ── Optional Tool interface extensions ─────────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind === 'pivot' || this.stage.kind === 'ref'
  }

  /**
   * Keep the protractor a constant on-screen size regardless of camera
   * distance — called from the Viewport render loop every frame (feature-
   * detected via `'updateDiskScale' in tool`). No-op when no disk is shown.
   */
  updateDiskScale(camera: THREE.Camera): void {
    if (this.previewDisk === null) return
    const dist = camera.position.distanceTo(this.previewDisk.position)
    this.previewDisk.scale.setScalar(DISK_SCREEN_K * dist)
  }

  // ── Tool interface ──────────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'ref') {
      const { pivot, previewMesh } = this.stage
      const axis = this._effectiveAxis()
      const cursorPoint = rayPlaneIntersect(ray.origin, ray.direction, pivot, axis)
      if (cursorPoint === null) {
        // Ray parallel to the rotation plane — hold the previous delta.
        if (previewMesh !== null) this._applyPreviewRotation(previewMesh, pivot, axis, this.stage.lastDelta)
        this._refreshDisk()
        this._reportAngleOrTyped(this.stage.lastDelta)
        return
      }

      const refVec: Vec3 = [
        this.stage.refPoint[0] - pivot[0], this.stage.refPoint[1] - pivot[1], this.stage.refPoint[2] - pivot[2],
      ]
      const cursorVec: Vec3 = [
        cursorPoint[0] - pivot[0], cursorPoint[1] - pivot[1], cursorPoint[2] - pivot[2],
      ]
      const raw = signedAngleAboutAxis(axis[0], axis[1], axis[2], refVec[0], refVec[1], refVec[2], cursorVec[0], cursorVec[1], cursorVec[2])
      const delta = snapAngleDeg(raw, SNAP_DEG)
      this.stage.lastDelta = delta

      if (previewMesh !== null) this._applyPreviewRotation(previewMesh, pivot, axis, delta)
      this._refreshDisk()
      this._reportAngleOrTyped(delta)
      return
    }

    // idle / pivot: keep the protractor centered under the cursor (idle) or at
    // the pivot (pivot), oriented to the inferred/locked axis.
    if (snap === null) return
    if (this.stage.kind === 'idle') {
      this.lastSnapPoint = [snap.x, snap.y, snap.z]
      this.candidateNormal = this._resolveAxis(snap, ray)
    }
    this._refreshDisk()
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      let nodes = this.selection
      if (nodes.length === 0 && this.acquireSelection !== null) {
        // Empty selection: auto-select whatever the click landed on and
        // start the rotation on it in the same gesture.
        const acquired = this.acquireSelection(ray)
        if (acquired !== null && acquired.length > 0) {
          this.selection = acquired
          nodes = acquired
        }
      }
      if (nodes.length === 0) {
        this.onToast('Click an object to rotate it')
        return
      }
      const pivot: Vec3 = [snap.x, snap.y, snap.z]
      // Refresh inference from the click snap so the captured axis reflects the
      // exact face/edge under the cursor even if no move preceded this click.
      this.candidateNormal = this._resolveAxis(snap, ray)
      const axis = this._effectiveAxis()
      this.typed = ''
      this.stage = { kind: 'pivot', nodes, pivot, axis }
      this.lastSnapPoint = pivot
      this._refreshDisk()
      this.onMeasurementCb('')
    } else if (this.stage.kind === 'pivot') {
      const { nodes, pivot, axis } = this.stage
      const refPoint: Vec3 = [snap.x, snap.y, snap.z]
      // Ignore a reference that coincides with the pivot, or lies on the axis
      // through it: its projection into the rotation plane is ~zero, which would
      // freeze the sweep at 0° with no feedback. Wait for a usable reference.
      const effAxis = this._effectiveAxis()
      const baseline = normalize3(projectOntoPlane(
        refPoint[0] - pivot[0], refPoint[1] - pivot[1], refPoint[2] - pivot[2],
        effAxis[0], effAxis[1], effAxis[2],
      ))
      if (baseline === null) return
      const previewMesh = this._buildPreview(nodes)
      if (previewMesh !== null) {
        this.preview.add(previewMesh)
      }
      this.stage = { kind: 'ref', nodes, pivot, axis, refPoint, previewMesh, lastDelta: 0 }
      this.typed = ''
      this._refreshDisk()
      this.onMeasurementCb('0.0°')
    } else if (this.stage.kind === 'ref') {
      const { nodes, pivot, lastDelta } = this.stage
      const delta = lastDelta
      const axis = this._effectiveAxis()

      this._resetToIdle()

      if (Math.abs(delta) < 1e-9) {
        // No-op rotation
        return
      }

      this._commit(nodes, pivot, axis, delta)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    // Shift toggles the axis lock (SketchUp inference-lock convention — toggle,
    // not hold, matching Slice/Protractor). Guard autorepeat so a held Shift
    // doesn't flicker the lock on and off.
    if (ev.key === 'Shift') {
      if (!ev.repeat) {
        this.lockedNormal = this.lockedNormal === null
          ? (normalize3(this._effectiveAxis()) ?? WORLD_UP)
          : null
        this._afterAxisChange()
      }
      return
    }

    // Arrow keys force-lock a world axis (→ X, ← Y, ↑ Z; ↓ clears the lock and
    // returns to inference). Works from the idle phase on, so a cylinder's spin
    // axis can be set before the center is even placed.
    if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
      ev.preventDefault()
      const idx = arrowToAxis(ev.key) // → 0/X, ← 1/Y, ↑ 2/Z, ↓ null
      this.lockedNormal = idx === null ? null : WORLD_AXIS[idx]
      this._afterAxisChange()
      return
    }

    // Numeric VCB is only meaningful while sweeping (typing an angle).
    if (this.stage.kind !== 'ref') return

    if (ev.key === 'Enter') {
      const n = parseDistance(this.typed)
      if (n !== null) {
        // Degrees are unitless — commit directly, no metersFromUnit conversion.
        const theta = (n * Math.PI) / 180
        const { nodes, pivot } = this.stage
        const axis = this._effectiveAxis()
        this._resetToIdle()
        if (Math.abs(theta) > 1e-9) {
          this._commit(nodes, pivot, axis, theta)
        }
      }
      return
    }

    if (
      (ev.key >= '0' && ev.key <= '9') ||
      ev.key === '.' ||
      ev.key === '-' ||
      ev.key === 'Backspace'
    ) {
      this.typed = editNumericBuffer(this.typed, ev.key)
      this._reportTyped()
    }
  }

  cancel(): void {
    this._resetToIdle()
  }

  // ── Private helpers ─────────────────────────────────────────────────────────

  /** The effective rotation axis (unit): a Shift/arrow lock wins; otherwise the
   * inferred axis (idle) or the pivot-captured axis (pivot/ref). */
  private _effectiveAxis(): Vec3 {
    if (this.lockedNormal !== null) return this.lockedNormal
    if (this.stage.kind === 'pivot' || this.stage.kind === 'ref') return this.stage.axis
    return this.candidateNormal ?? WORLD_UP
  }

  /**
   * Resolve the rotation axis for a hover/click snap:
   *   1. Snap on a live world-Object face → its unit normal.
   *   2. Snap on an edge/axis with a direction → that direction (spin about it).
   *   3. Otherwise → the face under the cursor ray (so a click on a face's
   *      corner or edge still tilts the protractor to that face), else +Z.
   */
  private _resolveAxis(snap: Snap, ray: Ray): Vec3 {
    if (snap.elementKind === 'face' && snap.object !== undefined && snap.element !== undefined) {
      try {
        const n = this.wasmScene.face_normal(snap.object, snap.element)
        const normal = normalize3([n[0], n[1], n[2]])
        if (normal !== null) return normal
      } catch {
        // Not a live world-Object face (e.g. instanced geometry) — fall through.
      }
    }
    if (snap.direction !== undefined) {
      const d = normalize3(snap.direction)
      if (d !== null) return d
    }
    return this._pickFaceAxis(ray)
  }

  /**
   * Pick the face under the ray (if any) and return its unit normal. Falls back
   * to world +Z when no face is hit.
   */
  private _pickFaceAxis(ray: Ray): Vec3 {
    let pick: ReturnType<WasmScene['pick_face']> | undefined
    try {
      pick = this.wasmScene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (pick === undefined) return WORLD_UP
      const n = this.wasmScene.face_normal(pick.object(), pick.face())
      return normalize3([n[0], n[1], n[2]]) ?? WORLD_UP
    } catch {
      return WORLD_UP
    } finally {
      pick?.free()
    }
  }

  /** Rotate an in-plane vector about `axis` by `theta` (radians). */
  private _rotateVec(v: Vec3, axis: Vec3, theta: number): Vec3 {
    const a = rotationAxisAffine(axis[0], axis[1], axis[2], theta)
    return [
      a[0] * v[0] + a[1] * v[1] + a[2] * v[2],
      a[4] * v[0] + a[5] * v[1] + a[6] * v[2],
      a[8] * v[0] + a[9] * v[1] + a[10] * v[2],
    ]
  }

  /** Report the angle, prefixed with an axis tag when the axis is world-aligned. */
  private _reportAngle(deltaRad: number): void {
    const deg = (deltaRad * 180) / Math.PI
    this.onMeasurementCb(`${this._axisTag()}${deg.toFixed(1)}°`)
  }

  /** Report the in-progress typed-angle buffer (with axis tag). */
  private _reportTyped(): void {
    this.onMeasurementCb(`${this._axisTag()}${this.typed}°`)
  }

  /**
   * While the user is mid-type, the typed buffer wins the readout (so it stays
   * visible as they type — the Viewport re-calls onPointerMove right after each
   * keystroke, which would otherwise overwrite it with the live cursor angle).
   */
  private _reportAngleOrTyped(deltaRad: number): void {
    if (this.typed !== '') {
      this._reportTyped()
      return
    }
    this._reportAngle(deltaRad)
  }

  /** "X "/"Y "/"Z " when the effective axis is world-aligned, else "". */
  private _axisTag(): string {
    const match = axisColorForDirection(this._effectiveAxis(), AXIS_SNAP_TOL_DOT)
    return match !== null ? `${AXIS_LABEL[match.axis]} ` : ''
  }

  private _buildPreview(nodes: NodeRef[]): THREE.Object3D | null {
    return buildSelectionPreview(this.wasmScene, this.objectsGroup, this.instanceGroupGetter, nodes)
  }

  /** Redraw the protractor for the current stage/axis. Reads the ghost from the
   * stage (in ref) but does not touch it. */
  private _refreshDisk(): void {
    const normal = normalize3(this._effectiveAxis()) ?? WORLD_UP
    const locked = this.lockedNormal !== null
    const match = axisColorForDirection(normal, AXIS_SNAP_TOL_DOT, axisColorsForTheme(getResolvedTheme()))
    const color = match !== null ? match.color : NEUTRAL_PREVIEW_COLOR

    let center: Vec3 | null
    const spokes: Spoke[] = []
    if (this.stage.kind === 'ref') {
      center = this.stage.pivot
      const baselineDir = normalize3([
        this.stage.refPoint[0] - this.stage.pivot[0],
        this.stage.refPoint[1] - this.stage.pivot[1],
        this.stage.refPoint[2] - this.stage.pivot[2],
      ])
      const baselineInPlane = baselineDir !== null
        ? normalize3(projectOntoPlane(baselineDir[0], baselineDir[1], baselineDir[2], normal[0], normal[1], normal[2]))
        : null
      if (baselineInPlane !== null) {
        spokes.push({ dir: baselineInPlane, color: BASELINE_ARM_COLOR })
        spokes.push({ dir: this._rotateVec(baselineInPlane, normal, this.stage.lastDelta), color })
      }
    } else if (this.stage.kind === 'pivot') {
      center = this.stage.pivot
    } else {
      center = this.lastSnapPoint
    }
    if (center === null) return
    this._updatePreviewDisk(center, normal, color, locked, spokes)
  }

  /**
   * Redraw the protractor after a Shift/arrow axis change: re-orient the ring
   * and, if a ghost is live (ref stage), re-apply the rotation about the new
   * axis so the ghost tracks the axis change without waiting for a move.
   */
  private _afterAxisChange(): void {
    this._refreshDisk()
    if (this.stage.kind === 'ref' && this.stage.previewMesh !== null) {
      const axis = this._effectiveAxis()
      this._applyPreviewRotation(this.stage.previewMesh, this.stage.pivot, axis, this.stage.lastDelta)
      this._reportAngleOrTyped(this.stage.lastDelta)
    }
  }

  private _commit(nodes: NodeRef[], pivot: Vec3, axis: Vec3, theta: number): void {
    try {
      const affine = rotateAboutPivotAxis(pivot[0], pivot[1], pivot[2], axis[0], axis[1], axis[2], theta)
      const affineF64 = affineToFloat64(affine)
      commitSelectionTransform(this.wasmScene, nodes, affineF64)
      this.onCommit(nodes)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
    }
  }

  private _resetToIdle(): void {
    this.stage = { kind: 'idle' }
    this.lockedNormal = null
    this.candidateNormal = null
    this.lastSnapPoint = null
    this.typed = ''
    this._clearDisk()
    clearPreview(this.preview)
    this.onMeasurementCb('')
  }

  /**
   * Update the ghost mesh by applying the rotation delta to its THREE.js matrix.
   * We reset and recompute rather than incrementally rotating so the preview
   * stays accurate on every pointer move.
   */
  private _applyPreviewRotation(mesh: THREE.Object3D, pivot: Vec3, axis: Vec3, theta: number): void {
    mesh.position.set(0, 0, 0)
    mesh.rotation.set(0, 0, 0)
    mesh.updateMatrix()

    const affine = rotateAboutPivotAxis(pivot[0], pivot[1], pivot[2], axis[0], axis[1], axis[2], theta)
    const m4 = new THREE.Matrix4()
    m4.set(
      affine[0], affine[1], affine[2], affine[3],
      affine[4], affine[5], affine[6], affine[7],
      affine[8], affine[9], affine[10], affine[11],
      0, 0, 0, 1,
    )
    mesh.applyMatrix4(m4)
  }

  /**
   * Rebuild the protractor: a ring (LineLoop) centered at `center`, lying in the
   * plane ⊥ `normal`, colored `color`. When `locked`, render at full opacity
   * with a short normal-axis tick so the lock is obvious; otherwise render
   * lighter. `spokes` are in-plane unit directions drawn as radial arms (the
   * baseline and swept-angle arms during a sweep).
   */
  private _updatePreviewDisk(center: Vec3, normal: Vec3, color: number, locked: boolean, spokes: Spoke[]): void {
    this._clearDisk()

    const unitNormal = normalize3(normal) ?? WORLD_UP
    const { u, v } = planeBasis(unitNormal)

    const ringPts = new Float32Array(DISK_SEGMENTS * 3)
    for (let i = 0; i < DISK_SEGMENTS; i++) {
      const theta = (i / DISK_SEGMENTS) * Math.PI * 2
      const c = Math.cos(theta), s = Math.sin(theta)
      ringPts[i * 3 + 0] = DISK_UNIT_RADIUS * (c * u[0] + s * v[0])
      ringPts[i * 3 + 1] = DISK_UNIT_RADIUS * (c * u[1] + s * v[1])
      ringPts[i * 3 + 2] = DISK_UNIT_RADIUS * (c * u[2] + s * v[2])
    }
    const ringGeo = new THREE.BufferGeometry()
    ringGeo.setAttribute('position', new THREE.BufferAttribute(ringPts, 3))
    const ringMat = new THREE.LineBasicMaterial({
      color,
      depthTest: false,
      transparent: !locked,
      opacity: locked ? 1 : 0.5,
    })
    const ring = new THREE.LineLoop(ringGeo, ringMat)

    const group = new THREE.Group()
    group.position.set(center[0], center[1], center[2])
    // Placeholder scale — updateDiskScale() corrects it next render frame
    // (avoids a one-frame flash at the unit radius before the screen-constant
    // size is applied).
    group.scale.setScalar(DISK_SCREEN_K * 4) // ~4 m fallback distance
    group.add(ring)

    if (locked) {
      const tickPts = new Float32Array([
        0, 0, 0,
        unitNormal[0] * DISK_TICK_LENGTH,
        unitNormal[1] * DISK_TICK_LENGTH,
        unitNormal[2] * DISK_TICK_LENGTH,
      ])
      const tickGeo = new THREE.BufferGeometry()
      tickGeo.setAttribute('position', new THREE.BufferAttribute(tickPts, 3))
      const tickMat = new THREE.LineBasicMaterial({ color, depthTest: false })
      group.add(new THREE.LineSegments(tickGeo, tickMat))
    }

    for (const spoke of spokes) {
      const armPts = new Float32Array([
        0, 0, 0,
        spoke.dir[0] * DISK_UNIT_RADIUS,
        spoke.dir[1] * DISK_UNIT_RADIUS,
        spoke.dir[2] * DISK_UNIT_RADIUS,
      ])
      const armGeo = new THREE.BufferGeometry()
      armGeo.setAttribute('position', new THREE.BufferAttribute(armPts, 3))
      const armMat = new THREE.LineBasicMaterial({ color: spoke.color, depthTest: false })
      group.add(new THREE.LineSegments(armGeo, armMat))
    }

    this.preview.add(group)
    this.previewDisk = group
  }

  private _clearDisk(): void {
    if (this.previewDisk === null) return
    for (const child of this.previewDisk.children) {
      if (child instanceof THREE.LineLoop || child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    }
    this.preview.remove(this.previewDisk)
    this.previewDisk = null
  }
}
