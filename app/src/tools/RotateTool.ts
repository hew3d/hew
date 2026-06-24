/**
 * RotateTool — SketchUp-style three-click object rotation about an
 * arbitrary axis (the "protractor" tool).
 *
 * Gesture (three-click):
 *   1. First click  : set the pivot point (snapped). The rotation axis
 *                     defaults to the normal of the face under the pivot
 *                     (or world +Z if no face is hit).
 *   2. Second click : set the start-reference point (angle ref).
 *   3. Move         : rubber-band preview showing rotation angle (snapped to
 *                     15° increments unless a value is typed); ghost preview
 *                     via a THREE.js clone.
 *   4. Third click  : commit rotation by the live delta about pivot via
 *                     transform_object/transform_group/transform_instance.
 *   5. Esc          : cancel current stage.
 *
 * Axis lock (while in 'pivot' or 'ref' stage):
 *   ArrowRight → lock X    ArrowLeft → lock Y    ArrowUp → lock Z
 *   ArrowDown            → clear lock, revert to the face-derived axis
 *
 * Numeric VCB (while in 'ref' stage):
 *   Type digits / . / - to build a buffer shown as the angle measurement.
 *   Press Enter to commit that exact number of DEGREES (unitless — no
 *   length-unit conversion) about the effective axis.
 *
 * The rotation plane is the plane through the pivot whose normal is the
 * effective axis (face normal, unless an arrow-key axis lock overrides it).
 * Because that plane often has no snappable geometry (e.g. a vertical plane
 * floating in space), the live preview during 'ref' uses ray/plane
 * intersection rather than the resolved snap point.
 *
 * If no object is selected, shows a hint toast and stays idle.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  rotateAboutPivotAxis,
  signedAngleAboutAxis,
  snapAngleDeg,
  affineToFloat64,
} from './transformMath'
import { rayPlaneIntersect, parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'
import { buildPreviewClone, buildMultiPreviewClone, buildInstancePreviewClone, buildSketchPreviewClone, clearPreview } from './transformPreview'
import { arrowToAxis, editNumericBuffer, parseDistance } from './moveInput'
import type { NodeRef } from '../panels/treeModel'

export type OnRotateCommit = (node: NodeRef) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'pivot'
      node: NodeRef
      pivot: [number, number, number]
      /** Unit axis derived from the face under the pivot (or world +Z). */
      faceAxis: [number, number, number]
    }
  | {
      kind: 'ref'
      node: NodeRef
      pivot: [number, number, number]
      faceAxis: [number, number, number]
      /** The 3D reference point (2nd click), stored so the reference vector
       * re-projects correctly if the effective axis changes mid-gesture. */
      refPoint: [number, number, number]
      previewMesh: THREE.Object3D | null
      /** Last computed delta (radians) — held steady when the cursor ray is
       * parallel to the rotation plane. */
      lastDelta: number
    }

const SNAP_DEG = 15
const WORLD_AXIS: Record<0 | 1 | 2, [number, number, number]> = {
  0: [1, 0, 0],
  1: [0, 1, 0],
  2: [0, 0, 1],
}
const AXIS_LABEL: Record<0 | 1 | 2, string> = { 0: 'X', 1: 'Y', 2: 'Z' }

export class RotateTool implements Tool {
  readonly name = 'Rotate'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnRotateCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement
  private selectedNode: NodeRef | null = null
  private objectsGroup: THREE.Group | null = null
  private instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null

  /** Current axis lock: 0=X, 1=Y, 2=Z, null=use faceAxis */
  private axisLock: 0 | 1 | 2 | null = null
  /** VCB buffer — raw string being typed by the user */
  private typed: string = ''

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selectedNode: NodeRef | null,
    onCommit: OnRotateCommit,
    onToast: OnToast,
    instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.objectsGroup = objectsGroup
    this.selectedNode = selectedNode
    this.onCommit = onCommit
    this.onToast = onToast
    this.instanceGroupGetter = instanceGroupGetter
    this.onMeasurementCb = onMeasurement
  }

  // ── Optional Tool interface extensions ─────────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind === 'pivot' || this.stage.kind === 'ref'
  }

  // ── Tool interface ──────────────────────────────────────────────────────────

  onPointerMove(_snap: Snap | null, ray: Ray): void {
    if (this.stage.kind !== 'ref') return
    const { pivot, refPoint, previewMesh } = this.stage
    if (previewMesh === null) return

    const axis = this._effectiveAxis()
    const cursorPoint = rayPlaneIntersect(ray.origin, ray.direction, pivot, axis)
    if (cursorPoint === null) {
      // Ray parallel to the rotation plane — hold the previous delta.
      this._applyPreviewRotation(previewMesh, pivot, axis, this.stage.lastDelta)
      this._reportAngleOrTyped(this.stage.lastDelta)
      return
    }

    const refVec: [number, number, number] = [
      refPoint[0] - pivot[0], refPoint[1] - pivot[1], refPoint[2] - pivot[2],
    ]
    const cursorVec: [number, number, number] = [
      cursorPoint[0] - pivot[0], cursorPoint[1] - pivot[1], cursorPoint[2] - pivot[2],
    ]

    const raw = signedAngleAboutAxis(axis[0], axis[1], axis[2], refVec[0], refVec[1], refVec[2], cursorVec[0], cursorVec[1], cursorVec[2])
    const delta = snapAngleDeg(raw, SNAP_DEG)
    this.stage.lastDelta = delta

    this._applyPreviewRotation(previewMesh, pivot, axis, delta)
    this._reportAngleOrTyped(delta)
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      const node = this.selectedNode
      if (node === null) {
        this.onToast('Select an object first, then use Rotate')
        return
      }
      const pivot: [number, number, number] = [snap.x, snap.y, snap.z]
      const faceAxis = this._pickFaceAxis(ray)
      this.axisLock = null
      this.typed = ''
      this.stage = { kind: 'pivot', node, pivot, faceAxis }
      this.onMeasurementCb('')
    } else if (this.stage.kind === 'pivot') {
      const { node, pivot, faceAxis } = this.stage
      const refPoint: [number, number, number] = [snap.x, snap.y, snap.z]
      const previewMesh = this._buildPreview(node)
      if (previewMesh !== null) {
        this.preview.add(previewMesh)
      }
      this.stage = { kind: 'ref', node, pivot, faceAxis, refPoint, previewMesh, lastDelta: 0 }
      this.typed = ''
      this.onMeasurementCb('0.0°')
    } else if (this.stage.kind === 'ref') {
      const { node, pivot, lastDelta } = this.stage
      const delta = lastDelta
      const axis = this._effectiveAxis()

      this._resetToIdle()

      if (Math.abs(delta) < 1e-9) {
        // No-op rotation
        return
      }

      this._commit(node, pivot, axis, delta)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    if (this.stage.kind !== 'pivot' && this.stage.kind !== 'ref') return

    // ── Axis lock via arrow keys ──
    if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
      const requested = arrowToAxis(ev.key)
      this.axisLock = requested
      return
    }

    // Numeric VCB is only meaningful in the 'ref' stage (typing an angle).
    if (this.stage.kind !== 'ref') return

    if (ev.key === 'Enter') {
      const n = parseDistance(this.typed)
      if (n !== null) {
        // Degrees are unitless — commit directly, no metersFromUnit conversion.
        const theta = (n * Math.PI) / 180
        const { node, pivot } = this.stage
        const axis = this._effectiveAxis()
        this._resetToIdle()
        if (Math.abs(theta) > 1e-9) {
          this._commit(node, pivot, axis, theta)
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

  /** The effective rotation axis: arrow-key lock overrides the face-derived axis. */
  private _effectiveAxis(): [number, number, number] {
    if (this.stage.kind !== 'pivot' && this.stage.kind !== 'ref') return [0, 0, 1]
    return this._effectiveAxisFrom(this.stage)
  }

  private _effectiveAxisFrom(stage: { faceAxis: [number, number, number] }): [number, number, number] {
    if (this.axisLock !== null) return WORLD_AXIS[this.axisLock]
    return stage.faceAxis
  }

  /**
   * Pick the face under the ray (if any) and return its unit normal as the
   * default rotation axis. Falls back to world +Z when no face is hit.
   */
  private _pickFaceAxis(ray: Ray): [number, number, number] {
    let pick: ReturnType<WasmScene['pick_face']> | undefined
    try {
      pick = this.wasmScene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (pick === undefined) return [0, 0, 1]
      const objectHandle = pick.object()
      const faceHandle = pick.face()
      const n = this.wasmScene.face_normal(objectHandle, faceHandle)
      const nx = n[0], ny = n[1], nz = n[2]
      const len = Math.sqrt(nx * nx + ny * ny + nz * nz)
      if (len < 1e-12) return [0, 0, 1]
      return [nx / len, ny / len, nz / len]
    } catch {
      return [0, 0, 1]
    } finally {
      pick?.free()
    }
  }

  /** Report the angle measurement, prefixed with a locked-axis tag if set. */
  private _reportAngle(deltaRad: number): void {
    const deg = (deltaRad * 180) / Math.PI
    const tag = this.axisLock !== null ? `${AXIS_LABEL[this.axisLock]} ` : ''
    this.onMeasurementCb(`${tag}${deg.toFixed(1)}°`)
  }

  /** Report the in-progress typed-angle buffer (with locked-axis tag if set). */
  private _reportTyped(): void {
    const tag = this.axisLock !== null ? `${AXIS_LABEL[this.axisLock]} ` : ''
    this.onMeasurementCb(`${tag}${this.typed}°`)
  }

  /**
   * While the user is mid-type, the typed buffer wins the readout (so it
   * stays visible as they type — the Viewport re-calls onPointerMove right
   * after each keystroke, which would otherwise overwrite it with the live
   * cursor angle). Otherwise show the live cursor-derived angle.
   */
  private _reportAngleOrTyped(deltaRad: number): void {
    if (this.typed !== '') {
      this._reportTyped()
      return
    }
    this._reportAngle(deltaRad)
  }

  private _buildPreview(node: NodeRef): THREE.Object3D | null {
    if (node.kind === 'group') {
      const leafIds = Array.from(this.wasmScene.node_leaf_objects(1, node.id))
      return buildMultiPreviewClone(this.objectsGroup, leafIds)
    }
    if (node.kind === 'instance') {
      const group = this.instanceGroupGetter !== null ? this.instanceGroupGetter(node.id) : null
      return buildInstancePreviewClone(group)
    }
    if (node.kind === 'sketch') {
      return buildSketchPreviewClone(this.wasmScene.sketch_lines(node.id))
    }
    return buildPreviewClone(this.objectsGroup, node.id)
  }

  private _commit(
    node: NodeRef,
    pivot: [number, number, number],
    axis: [number, number, number],
    theta: number,
  ): void {
    try {
      const affine = rotateAboutPivotAxis(pivot[0], pivot[1], pivot[2], axis[0], axis[1], axis[2], theta)
      const affineF64 = affineToFloat64(affine)
      if (node.kind === 'group') {
        this.wasmScene.transform_group(node.id, affineF64)
      } else if (node.kind === 'instance') {
        this.wasmScene.transform_instance(node.id, affineF64)
      } else if (node.kind === 'sketch') {
        this.wasmScene.transform_sketch(node.id, affineF64)
      } else {
        this.wasmScene.transform_object(node.id, affineF64)
      }
      this.onCommit(node)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
    }
  }

  private _resetToIdle(): void {
    this.stage = { kind: 'idle' }
    this.axisLock = null
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')
  }

  /**
   * Update the preview mesh by applying rotation delta to its THREE.js matrix.
   * We reset and recompute rather than incrementally rotating so the preview
   * stays accurate on every pointer move.
   */
  private _applyPreviewRotation(
    mesh: THREE.Object3D,
    pivot: [number, number, number],
    axis: [number, number, number],
    theta: number,
  ): void {
    // Reset position; apply the affine transform directly.
    mesh.position.set(0, 0, 0)
    mesh.rotation.set(0, 0, 0)
    mesh.updateMatrix()

    // Build the 4×4 transform matrix from our affine
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

}
