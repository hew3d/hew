/**
 * ScaleTool — uniform scale from a click-drag, about the object's bounding-box
 * center.
 *
 * Gesture (two-click):
 *   1. First click  : record the base point (snapped); compute bounding-box
 *                     center from the object mesh positions.
 *   2. Move         : compute scale factor as |dest − center| / |base − center|;
 *                     update a THREE.js ghost preview.
 *   3. Second click : commit the uniform scale (one node → the per-kind
 *                     transform method; a multi-selection → one
 *                     transform_selection call, one undo step).
 *   4. Esc          : cancel.
 *
 * Scale factor is clamped to a minimum of 0.01 to avoid degenerate / reflection
 * results (the kernel rejects factor ≤ 0 with "Singular" / "Reflection").
 *
 * If nothing is selected, shows a hint toast and stays idle.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { scaleAboutCenter, affineToFloat64 } from './transformMath'
import { parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'
import { clearPreview } from './transformPreview'
import { commitSelectionTransform, buildSelectionPreview } from './transformSelection'
import { editNumericBuffer, parseDistance } from './moveInput'
import type { NodeRef } from '../panels/treeModel'

export type OnScaleCommit = (nodes: NodeRef[]) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

const MIN_SCALE = 0.01

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'dragging'
      nodes: NodeRef[]
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
  private onMeasurementCb: OnMeasurement
  private selection: NodeRef[] = []
  private objectsGroup: THREE.Group | null = null
  private instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null
  /** VCB buffer — raw string being typed by the user (unitless factor) */
  private typed: string = ''

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selection: NodeRef[],
    onCommit: OnScaleCommit,
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

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (this.stage.kind !== 'dragging' || snap === null) return
    const { center, baseDist, previewMesh } = this.stage
    if (previewMesh === null || baseDist < 1e-9) return

    const f = this._computeFactor(center, [snap.x, snap.y, snap.z], baseDist)
    this._applyPreviewScale(previewMesh, center, f)
    if (this.typed === '') {
      this.onMeasurementCb(`×${f.toFixed(2)}`)
    }
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      const nodes = this.selection
      if (nodes.length === 0) {
        this.onToast('Select an object first, then use Scale')
        return
      }

      // Bounding-box center to scale about. For a group or multi-selection
      // this is the aggregate center across every target, so the whole
      // selection scales as a unit rather than pivoting on one member.
      const center: [number, number, number] =
        this._selectionCenter(nodes) ?? [snap.x, snap.y, snap.z]

      const baseDist = this._dist(center, [snap.x, snap.y, snap.z])
      const previewMesh = this._buildPreview(nodes)
      if (previewMesh !== null) {
        this.preview.add(previewMesh)
      }

      this.stage = { kind: 'dragging', nodes, center, baseDist, previewMesh }
      this.onMeasurementCb('×1.00')
    } else if (this.stage.kind === 'dragging') {
      const { nodes, center, baseDist } = this.stage
      const f = this._computeFactor(center, [snap.x, snap.y, snap.z], baseDist)

      this.stage = { kind: 'idle' }
      this.typed = ''
      clearPreview(this.preview)
      this.onMeasurementCb('')
      this._commit(nodes, center, f)
    }
  }

  capturingInput(): boolean {
    return this.stage.kind === 'dragging'
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    if (this.stage.kind !== 'dragging') return

    // ── Numeric VCB (a unitless scale factor — no unit conversion) ──
    if (ev.key === 'Enter') {
      const n = parseDistance(this.typed)
      if (n !== null && n > 0) {
        this._commitFromTyped(Math.max(n, MIN_SCALE))
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
      this.onMeasurementCb(`×${this.typed}`)
    }
  }

  cancel(): void {
    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')
  }

  /** Commit the scale from the typed VCB buffer, then reset to idle. */
  private _commitFromTyped(f: number): void {
    if (this.stage.kind !== 'dragging') return
    const { nodes, center } = this.stage

    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')
    this._commit(nodes, center, f)
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

  private _buildPreview(nodes: NodeRef[]): THREE.Object3D | null {
    return buildSelectionPreview(this.wasmScene, this.objectsGroup, this.instanceGroupGetter, nodes)
  }

  /**
   * World-space bounding-box center of the whole selection, computed from
   * the rendered meshes — pose-correct for instances (their definition-local
   * geometry is mapped through the instance group's matrix) and free of FFI
   * buffer copies. Free sketches contribute their world-space line
   * endpoints. Null when nothing in the selection has geometry.
   */
  private _selectionCenter(nodes: NodeRef[]): [number, number, number] | null {
    const box = new THREE.Box3()
    const pt = new THREE.Vector3()
    for (const node of nodes) {
      if (node.kind === 'sketch-edge' || node.kind === 'sketch-curve') {
        continue // not transformable — contributes nothing to the center
      }
      if (node.kind === 'sketch-island' && node.sketch !== undefined) {
        const lines = this.wasmScene.sketch_island_lines(node.sketch, node.id)
        for (let i = 0; i + 2 < lines.length; i += 3) {
          box.expandByPoint(pt.set(lines[i], lines[i + 1], lines[i + 2]))
        }
        continue
      }
      if (node.kind === 'sketch') {
        const lines = this.wasmScene.sketch_lines(node.id)
        for (let i = 0; i + 2 < lines.length; i += 3) {
          box.expandByPoint(pt.set(lines[i], lines[i + 1], lines[i + 2]))
        }
      } else if (node.kind === 'instance') {
        const group = this.instanceGroupGetter !== null ? this.instanceGroupGetter(node.id) : null
        if (group !== null) box.expandByObject(group)
      } else {
        const leafIds = node.kind === 'group'
          ? Array.from(this.wasmScene.node_leaf_objects(1, node.id))
          : [node.id]
        for (const id of leafIds) {
          const objGroup = this.objectsGroup?.getObjectByName(`Object_${id}`)
          if (objGroup !== undefined) box.expandByObject(objGroup)
        }
      }
    }
    if (box.isEmpty()) return null
    const c = box.getCenter(pt)
    return [c.x, c.y, c.z]
  }

  private _commit(
    nodes: NodeRef[],
    center: [number, number, number],
    f: number,
  ): void {
    if (Math.abs(f - 1) < 1e-9) {
      // Near-identity scale; skip the kernel call
      this.onCommit(nodes)
      return
    }
    try {
      const affine = scaleAboutCenter(center[0], center[1], center[2], f)
      const affineF64 = affineToFloat64(affine)
      commitSelectionTransform(this.wasmScene, nodes, affineF64)
      this.onCommit(nodes)
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
