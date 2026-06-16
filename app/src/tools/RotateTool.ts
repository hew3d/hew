/**
 * RotateTool — SketchUp-style three-click object rotation about world +Z.
 *
 * Gesture (three-click):
 *   1. First click  : set the pivot point (snapped).
 *   2. Second click : set the start-reference direction (angle ref).
 *   3. Move         : rubber-band preview showing rotation angle (snapped to
 *                     15° increments); ghost preview via a THREE.js clone.
 *   4. Third click  : commit rotation by (angle − refAngle) about pivot via
 *                     transform_object.
 *   5. Esc          : cancel current stage.
 *
 * v1: rotates about world +Z through the pivot (ground-plane spin).
 * The resulting affine is: T(pivot) · R_Z(θ) · T(−pivot).
 *
 * If no object is selected, shows a hint toast and stays idle.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { rotateAboutPivotZ, snapAngleDeg, angleFromPivot, affineToFloat64 } from './transformMath'
import { parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'
import { buildPreviewClone, clearPreview } from './transformPreview'

export type OnRotateCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void

type Stage =
  | { kind: 'idle' }
  | { kind: 'pivot'; objectId: bigint; pivot: [number, number, number] }
  | {
      kind: 'ref'
      objectId: bigint
      pivot: [number, number, number]
      refAngle: number
      previewMesh: THREE.Object3D | null
    }

const SNAP_DEG = 15

export class RotateTool implements Tool {
  readonly name = 'Rotate'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnRotateCommit
  private onToast: OnToast
  private selectedObjectId: bigint | null = null
  private objectsGroup: THREE.Group | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selectedObjectId: bigint | null,
    onCommit: OnRotateCommit,
    onToast: OnToast,
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.objectsGroup = objectsGroup
    this.selectedObjectId = selectedObjectId
    this.onCommit = onCommit
    this.onToast = onToast
  }

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (this.stage.kind !== 'ref' || snap === null) return
    const { pivot, refAngle, previewMesh } = this.stage
    if (previewMesh === null) return

    const currentAngle = angleFromPivot(pivot[0], pivot[1], snap.x, snap.y)
    const delta = snapAngleDeg(currentAngle - refAngle, SNAP_DEG)

    // Apply rotation to the preview clone
    this._applyPreviewRotation(previewMesh, pivot, delta)
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      const objectId = this.selectedObjectId
      if (objectId === null) {
        this.onToast('Select an object first, then use Rotate')
        return
      }
      const pivot: [number, number, number] = [snap.x, snap.y, snap.z]
      this.stage = { kind: 'pivot', objectId, pivot }
    } else if (this.stage.kind === 'pivot') {
      const { objectId, pivot } = this.stage
      const refAngle = angleFromPivot(pivot[0], pivot[1], snap.x, snap.y)
      const previewMesh = buildPreviewClone(this.objectsGroup, objectId)
      if (previewMesh !== null) {
        this.preview.add(previewMesh)
      }
      this.stage = { kind: 'ref', objectId, pivot, refAngle, previewMesh }
    } else if (this.stage.kind === 'ref') {
      const { objectId, pivot, refAngle, previewMesh } = this.stage
      const currentAngle = angleFromPivot(pivot[0], pivot[1], snap.x, snap.y)
      const delta = snapAngleDeg(currentAngle - refAngle, SNAP_DEG)

      this.stage = { kind: 'idle' }
      if (previewMesh !== null) {
        // Remove from preview group before _clearPreview disposes everything
      }
      clearPreview(this.preview)

      if (Math.abs(delta) < 1e-9) {
        // No-op rotation
        return
      }

      this._commit(objectId, pivot, delta)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
    }
  }

  cancel(): void {
    this.stage = { kind: 'idle' }
    clearPreview(this.preview)
  }

  private _commit(
    objectId: bigint,
    pivot: [number, number, number],
    theta: number,
  ): void {
    try {
      const affine = rotateAboutPivotZ(pivot[0], pivot[1], pivot[2], theta)
      this.wasmScene.transform_object(objectId, affineToFloat64(affine))
      this.onCommit(objectId)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
    }
  }

  /**
   * Update the preview mesh by applying rotation delta to its THREE.js matrix.
   * We reset and recompute rather than incrementally rotating so the preview
   * stays accurate on every pointer move.
   */
  private _applyPreviewRotation(
    mesh: THREE.Object3D,
    pivot: [number, number, number],
    theta: number,
  ): void {
    // Reset position; apply the affine transform directly.
    mesh.position.set(0, 0, 0)
    mesh.rotation.set(0, 0, 0)
    mesh.updateMatrix()

    // Build the 4×4 transform matrix from our affine
    const affine = rotateAboutPivotZ(pivot[0], pivot[1], pivot[2], theta)
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
