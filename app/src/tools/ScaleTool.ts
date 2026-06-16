/**
 * ScaleTool — uniform scale from a click-drag, about the object's bounding-box
 * center.
 *
 * Gesture (two-click):
 *   1. First click  : record the base point (snapped); compute bounding-box
 *                     center from the object mesh positions.
 *   2. Move         : compute scale factor as |dest − center| / |base − center|;
 *                     update a THREE.js ghost preview.
 *   3. Second click : commit the uniform scale via transform_object.
 *   4. Esc          : cancel.
 *
 * Scale factor is clamped to a minimum of 0.01 to avoid degenerate / reflection
 * results (the kernel rejects factor ≤ 0 with "Singular" / "Reflection").
 *
 * If no object is selected, shows a hint toast and stays idle.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { scaleAboutCenter, meshBoundingBoxCenter, affineToFloat64 } from './transformMath'
import { parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'
import { buildPreviewClone, clearPreview } from './transformPreview'

export type OnScaleCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void

const MIN_SCALE = 0.01

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'dragging'
      objectId: bigint
      center: [number, number, number]
      baseDist: number
      previewMesh: THREE.Object3D | null
    }

export class ScaleTool implements Tool {
  readonly name = 'Scale'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnScaleCommit
  private onToast: OnToast
  private selectedObjectId: bigint | null = null
  private objectsGroup: THREE.Group | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selectedObjectId: bigint | null,
    onCommit: OnScaleCommit,
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
    if (this.stage.kind !== 'dragging' || snap === null) return
    const { center, baseDist, previewMesh } = this.stage
    if (previewMesh === null || baseDist < 1e-9) return

    const f = this._computeFactor(center, [snap.x, snap.y, snap.z], baseDist)
    this._applyPreviewScale(previewMesh, center, f)
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      const objectId = this.selectedObjectId
      if (objectId === null) {
        this.onToast('Select an object first, then use Scale')
        return
      }

      // Get the mesh positions to compute the bounding-box center
      let center: [number, number, number] = [0, 0, 0]
      try {
        const mesh = this.wasmScene.object_mesh(objectId)
        try {
          center = meshBoundingBoxCenter(mesh.positions())
        } finally {
          mesh.free()
        }
      } catch {
        // If we can't get mesh data, fall back to snap point as center
        center = [snap.x, snap.y, snap.z]
      }

      const baseDist = this._dist(center, [snap.x, snap.y, snap.z])
      const previewMesh = buildPreviewClone(this.objectsGroup, objectId)
      if (previewMesh !== null) {
        this.preview.add(previewMesh)
      }

      this.stage = { kind: 'dragging', objectId, center, baseDist, previewMesh }
    } else if (this.stage.kind === 'dragging') {
      const { objectId, center, baseDist } = this.stage
      const f = this._computeFactor(center, [snap.x, snap.y, snap.z], baseDist)

      this.stage = { kind: 'idle' }
      clearPreview(this.preview)
      this._commit(objectId, center, f)
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

  private _dist(a: [number, number, number], b: [number, number, number]): number {
    const dx = b[0] - a[0]
    const dy = b[1] - a[1]
    const dz = b[2] - a[2]
    return Math.sqrt(dx * dx + dy * dy + dz * dz)
  }

  private _computeFactor(
    center: [number, number, number],
    dest: [number, number, number],
    baseDist: number,
  ): number {
    if (baseDist < 1e-9) return 1
    const destDist = this._dist(center, dest)
    const f = destDist / baseDist
    return Math.max(f, MIN_SCALE)
  }

  private _commit(
    objectId: bigint,
    center: [number, number, number],
    f: number,
  ): void {
    if (Math.abs(f - 1) < 1e-9) {
      // Near-identity scale; skip the kernel call
      this.onCommit(objectId)
      return
    }
    try {
      const affine = scaleAboutCenter(center[0], center[1], center[2], f)
      this.wasmScene.transform_object(objectId, affineToFloat64(affine))
      this.onCommit(objectId)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
    }
  }

  private _applyPreviewScale(
    mesh: THREE.Object3D,
    center: [number, number, number],
    f: number,
  ): void {
    // Reset accumulated transform, then apply the new one
    mesh.position.set(0, 0, 0)
    mesh.rotation.set(0, 0, 0)
    mesh.scale.set(1, 1, 1)
    mesh.updateMatrix()

    const affine = scaleAboutCenter(center[0], center[1], center[2], f)
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
