/**
 * PushPullTool — hover-highlight + drag to extrude/push-pull.
 *
 * Gesture (two-click mode):
 *   1. Hover: snap() → highlight face when on-face snap with element
 *   2. First click: enter drag mode, record anchor point + normal axis
 *   3. Move: project ray onto normal axis from anchor → ghost preview height
 *   4. Second click: commit extrude_region or push_pull
 *   5. Esc: cancel
 *
 * Calls onCommit after each successful commit.
 * Throws kernel "CODE: message" errors as toasts via onToast.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import { intersectGroundPlane } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { projectRayOntoAxis, parseKernelErrorCode, kernelErrorMessage, pointInPolygonXY, polygonAreaXY } from '../viewport/geoHelpers'

export type PushPullTarget =
  | { kind: 'region'; sketchHandle: bigint; regionHandle: bigint; normal: [number, number, number] }
  | { kind: 'face'; objectHandle: bigint; faceHandle: bigint; normal: [number, number, number] }

export type OnPushPullCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'dragging'
      target: PushPullTarget
      anchor: [number, number, number]
      /** Last computed signed distance */
      distance: number
    }

export class PushPullTool implements Tool {
  readonly name = 'Push/Pull'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnPushPullCommit
  private onToast: OnToast

  /** The snap last seen on hover (for highlight logic) */
  lastSnap: Snap | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnPushPullCommit,
    onToast: OnToast,
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    this.lastSnap = snap

    if (this.stage.kind === 'dragging') {
      const { target, anchor } = this.stage

      // Always project the ray onto the normal axis during drag — snap may be
      // null when the cursor is over empty space, but we still need the distance.
      const distance = projectRayOntoAxis(
        ray.origin,
        ray.direction,
        anchor,
        target.normal,
      )
      this.stage = { ...this.stage, distance }
      this._drawGhostPreview(anchor, target.normal, distance)
    }
    // Hover highlight is handled via CueLayer for on-face snaps (M1 shortcut
    // per docs/DEVELOPMENT.md: show snap marker at face location; per-face highlight
    // requires face→triangle table in MeshJs which is a WASM_API addendum).
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'idle') {
      let target: PushPullTarget | null = null
      let anchor: [number, number, number] = [0, 0, 0]

      // --- Path A: ray-cast for the nearest object face (ignores snap priority) ---
      // pick_face bypasses the drawing snap bias toward vertices/edges, so it
      // reliably returns the surface under the cursor even when snap prefers a
      // nearby endpoint.  We call this FIRST; Path B only fires when no object
      // face is hit (bare ground or no objects yet).
      const pick = this.wasmScene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (pick !== undefined) {
        try {
          const objectHandle = pick.object()
          // Inside an editing context, only the entered object is editable —
          // ignore faces of other objects so isolated editing can't disturb
          // neighbors. Top level (null) keeps the unrestricted behavior.
          if (this._activeContext === null || objectHandle === this._activeContext) {
            const faceHandle = pick.face()
            const normalArr = this.wasmScene.face_normal(objectHandle, faceHandle)
            const normal: [number, number, number] = [normalArr[0], normalArr[1], normalArr[2]]
            // Prefer the snap position as anchor (snapped to a real point on the
            // surface); fall back to ground hit, then ray origin.
            if (snap !== null) {
              anchor = [snap.x, snap.y, snap.z]
            } else {
              const hit = intersectGroundPlane(ray)
              anchor = hit !== null ? [hit.x, hit.y, hit.z] : [...ray.origin]
            }
            target = { kind: 'face', objectHandle, faceHandle, normal }
          }
        } finally {
          pick.free()
        }
      }

      // --- Path B: no object face hit — try picking a sketch region ---
      // Only reached when pick_face returns undefined (bare ground click, or
      // no objects in scene yet).
      // Region extrusion is a top-level act; suppress it inside a context.
      if (target === null && this._activeContext === null && this._sketchHandle !== null) {
        // Intersect the ray with the ground plane to get the hit point
        const hit = intersectGroundPlane(ray)
        if (hit !== null) {
          // Use snap position if we have a snap, otherwise use ground hit
          anchor = snap !== null ? [snap.x, snap.y, snap.z] : [hit.x, hit.y, hit.z]
          const px = hit.x
          const py = hit.y
          const sketchHandle = this._sketchHandle
          const regionHandles = this.wasmScene.sketch_regions(sketchHandle)

          // Among all regions whose outer boundary contains the hit point,
          // pick the one with the smallest outer-polygon area. This ensures a
          // click inside a nested inner rectangle selects that rectangle's
          // region rather than the enclosing ring's region (both contain the
          // point, but the inner region is smaller).
          let bestHandle: bigint | null = null
          let bestArea = Infinity
          for (let i = 0; i < regionHandles.length; i++) {
            const regionHandle = regionHandles[i]
            const boundary = this.wasmScene.region_boundary(sketchHandle, regionHandle)
            if (pointInPolygonXY(px, py, boundary)) {
              const area = polygonAreaXY(boundary)
              if (area < bestArea) {
                bestArea = area
                bestHandle = regionHandle
              }
            }
          }
          if (bestHandle !== null) {
            target = {
              kind: 'region',
              sketchHandle,
              regionHandle: bestHandle,
              normal: [0, 0, 1], // ground plane normal
            }
          }
        }
      }

      if (target === null) return

      this.stage = { kind: 'dragging', target, anchor, distance: 0 }
    } else if (this.stage.kind === 'dragging') {
      // Second click: commit with current distance
      const { target, anchor, distance } = this.stage
      this.stage = { kind: 'idle' }
      this._clearPreview()

      // Project ray onto axis at click point for final distance measurement
      const finalDistance = projectRayOntoAxis(
        ray.origin,
        ray.direction,
        anchor,
        target.normal,
      )
      this._commit(target, finalDistance === 0 ? distance : finalDistance)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
    }
  }

  cancel(): void {
    this.stage = { kind: 'idle' }
    this._clearPreview()
    this.lastSnap = null
  }

  /** Set the currently active sketch handle (for region extrusion) */
  private _sketchHandle: bigint | null = null
  setSketchHandle(handle: bigint | null): void {
    this._sketchHandle = handle
  }

  /** Set the active editing context (entered object), or null for top level.
   *  When set, push/pull only acts on that object's faces ( scoped editing). */
  private _activeContext: bigint | null = null
  setActiveContext(objectId: bigint | null): void {
    this._activeContext = objectId
  }

  /**
   * Set the active component context: when the user has double-clicked into an
   * instance, push/pull routes face operations through `push_pull_in_component`
   * instead of `push_pull`. `componentId` is the definition handle (from
   * `instance_def`), or null for normal (non-instance) context.
   */
  private _activeComponent: bigint | null = null
  setComponentContext(componentId: bigint | null): void {
    this._activeComponent = componentId
  }

  private _commit(target: PushPullTarget, distance: number): void {
    if (Math.abs(distance) < 1e-6) {
      this.onToast('Move more before committing push/pull')
      return
    }

    try {
      if (target.kind === 'region') {
        const objectId = this.wasmScene.extrude_region(
          target.sketchHandle,
          target.regionHandle,
          distance,
        )
        this.onCommit(objectId)
      } else {
        // Route through push_pull_in_component when inside a component editing context.
        const report = this._activeComponent !== null
          ? this.wasmScene.push_pull_in_component(
              this._activeComponent,
              target.objectHandle,
              target.faceHandle,
              distance,
            )
          : this.wasmScene.push_pull(
              target.objectHandle,
              target.faceHandle,
              distance,
            )
        try {
          this.onCommit(target.objectHandle)
        } finally {
          report.free()
        }
      }
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
    }
  }

  private _drawGhostPreview(
    anchor: [number, number, number],
    normal: [number, number, number],
    distance: number,
  ): void {
    this._clearPreview()

    if (Math.abs(distance) < 1e-6) return

    // Draw a simple arrow/line from anchor along normal to show extrusion distance
    const [ax, ay, az] = anchor
    const [nx, ny, nz] = normal

    const tip: [number, number, number] = [
      ax + nx * distance,
      ay + ny * distance,
      az + nz * distance,
    ]

    const pts = new Float32Array([ax, ay, az, ...tip])
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
    const mat = new THREE.LineBasicMaterial({
      color: 0xee8800,
      depthTest: false,
    })
    const line = new THREE.LineSegments(geo, mat)
    line.renderOrder = 996
    this.preview.add(line)

    // Also render a small cross at the tip
    const s = 0.05
    const crossPts = new Float32Array([
      tip[0] - s, tip[1], tip[2],  tip[0] + s, tip[1], tip[2],
      tip[0], tip[1] - s, tip[2],  tip[0], tip[1] + s, tip[2],
    ])
    const crossGeo = new THREE.BufferGeometry()
    crossGeo.setAttribute('position', new THREE.BufferAttribute(crossPts, 3))
    const crossMat = new THREE.LineBasicMaterial({ color: 0xee8800, depthTest: false })
    const cross = new THREE.LineSegments(crossGeo, crossMat)
    cross.renderOrder = 996
    this.preview.add(cross)
  }

  private _clearPreview(): void {
    this.preview.traverse((child) => {
      if (child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    })
    this.preview.clear()
  }
}
