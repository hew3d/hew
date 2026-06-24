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
import { buildPreviewClone, buildMultiPreviewClone, buildInstancePreviewClone, buildSketchPreviewClone, clearPreview } from './transformPreview'
import { editNumericBuffer, parseDistance } from './moveInput'
import type { NodeRef } from '../panels/treeModel'

export type OnScaleCommit = (node: NodeRef) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

const MIN_SCALE = 0.01

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'dragging'
      node: NodeRef
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
  private selectedNode: NodeRef | null = null
  private objectsGroup: THREE.Group | null = null
  private instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null
  /** VCB buffer — raw string being typed by the user (unitless factor) */
  private typed: string = ''

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selectedNode: NodeRef | null,
    onCommit: OnScaleCommit,
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
      const node = this.selectedNode
      if (node === null) {
        this.onToast('Select an object first, then use Scale')
        return
      }

      // Bounding-box center to scale about. For a group this is the aggregate
      // center across every leaf object, so the whole group scales as a unit
      // rather than pivoting on one member.
      const center: [number, number, number] =
        this._nodeCenter(node) ?? [snap.x, snap.y, snap.z]

      const baseDist = this._dist(center, [snap.x, snap.y, snap.z])
      const previewMesh = this._buildPreview(node)
      if (previewMesh !== null) {
        this.preview.add(previewMesh)
      }

      this.stage = { kind: 'dragging', node, center, baseDist, previewMesh }
      this.onMeasurementCb('×1.00')
    } else if (this.stage.kind === 'dragging') {
      const { node, center, baseDist } = this.stage
      const f = this._computeFactor(center, [snap.x, snap.y, snap.z], baseDist)

      this.stage = { kind: 'idle' }
      this.typed = ''
      clearPreview(this.preview)
      this.onMeasurementCb('')
      this._commit(node, center, f)
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
    const { node, center } = this.stage

    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')
    this._commit(node, center, f)
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

  /**
   * Bounding-box center to scale about: the node's own mesh for an object, or
   * the aggregate bbox center across all leaf meshes for a group (so a group
   * scales about its overall center, not one member's). Null if no mesh data.
   */
  private _nodeCenter(node: NodeRef): [number, number, number] | null {
    if (node.kind === 'sketch') {
      const lines = this.wasmScene.sketch_lines(node.id)
      if (lines.length === 0) return null
      const positions = lines instanceof Float32Array ? lines : new Float32Array(lines)
      return meshBoundingBoxCenter(positions)
    }
    if (node.kind === 'instance') {
      // For an instance, use the member objects' positions mapped through the pose.
      // Simplest: fetch member meshes and average their positions (definition-local).
      const componentId = this.wasmScene.instance_def(node.id)
      if (componentId === undefined) return null
      const memberIds = Array.from(this.wasmScene.component_member_objects(componentId))
      const chunks: Float32Array[] = []
      for (const id of memberIds) {
        let mesh
        try {
          mesh = this.wasmScene.object_mesh(id)
        } catch {
          continue
        }
        try {
          chunks.push(mesh.positions())
        } finally {
          mesh.free()
        }
      }
      if (chunks.length === 0) return null
      const total = chunks.reduce((n, c) => n + c.length, 0)
      const all = new Float32Array(total)
      let off = 0
      for (const c of chunks) {
        all.set(c, off)
        off += c.length
      }
      return meshBoundingBoxCenter(all)
    }
    const leafIds = node.kind === 'group'
      ? Array.from(this.wasmScene.node_leaf_objects(1, node.id))
      : [node.id]
    const chunks: Float32Array[] = []
    for (const id of leafIds) {
      let mesh
      try {
        mesh = this.wasmScene.object_mesh(id)
      } catch {
        continue
      }
      try {
        chunks.push(mesh.positions())
      } finally {
        mesh.free()
      }
    }
    if (chunks.length === 0) return null
    if (chunks.length === 1) return meshBoundingBoxCenter(chunks[0])
    const total = chunks.reduce((n, c) => n + c.length, 0)
    const all = new Float32Array(total)
    let off = 0
    for (const c of chunks) {
      all.set(c, off)
      off += c.length
    }
    return meshBoundingBoxCenter(all)
  }

  private _commit(
    node: NodeRef,
    center: [number, number, number],
    f: number,
  ): void {
    if (Math.abs(f - 1) < 1e-9) {
      // Near-identity scale; skip the kernel call
      this.onCommit(node)
      return
    }
    try {
      const affine = scaleAboutCenter(center[0], center[1], center[2], f)
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
