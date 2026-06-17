/**
 * PaintTool — hover-highlight the picked face, click to paint it with the
 * current material.
 *
 * Gesture (single-click mode):
 *   1. Hover: pick_face() → highlight the face under the cursor
 *   2. Click: call scene.paint_face(object, face, currentMaterialId)
 *   3. Esc: cancel hover state
 *
 * The current material is set via `setCurrentMaterial(id)` from the
 * MaterialPalette panel. Sentinel `BigInt(0xFFFFFFFFFFFFFFFF)` = default/unpaint.
 *
 * Face picking reuses the existing pick_face() path (same as PushPullTool).
 */

import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'

export type OnPaintCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void

/** `u64::MAX` as a BigInt — sentinel for "default / unpaint". */
export const MATERIAL_SENTINEL: bigint = BigInt('18446744073709551615')

export class PaintTool implements Tool {
  readonly name = 'Paint'

  private wasmScene: WasmScene
  private onCommit: OnPaintCommit
  private onToast: OnToast
  private currentMaterialId: bigint = MATERIAL_SENTINEL
  /** When true, the next click fills the whole object (base material). */
  private wholeObject = false

  /** The face currently hovered over (for highlight), or null. */
  hoveredObject: bigint | null = null
  hoveredFace: bigint | null = null

  /** The snap last seen on hover (for cue rendering). */
  lastSnap: Snap | null = null

  constructor(
    wasmScene: WasmScene,
    onCommit: OnPaintCommit,
    onToast: OnToast,
  ) {
    this.wasmScene = wasmScene
    this.onCommit = onCommit
    this.onToast = onToast
  }

  /** Set the active material id. `MATERIAL_SENTINEL` = unpaint. */
  setCurrentMaterial(id: bigint): void {
    this.currentMaterialId = id
  }

  getCurrentMaterial(): bigint {
    return this.currentMaterialId
  }

  /**
   * When `b` is true the next click sets the **object base material** via
   * `set_object_material` instead of painting a single face. Auto-resets to
   * false after one click.
   */
  setWholeObject(b: boolean): void {
    this.wholeObject = b
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    this.lastSnap = snap

    // Ray-cast for the nearest face to drive the hover highlight.
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )

    if (pick !== undefined) {
      try {
        this.hoveredObject = pick.object()
        this.hoveredFace = pick.face()
      } finally {
        pick.free()
      }
    } else {
      this.hoveredObject = null
      this.hoveredFace = null
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    // Re-pick at click position (move may have been slightly different).
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )

    if (pick === undefined) return

    const wholeObject = this.wholeObject
    this.wholeObject = false

    try {
      const objectHandle = pick.object()
      const faceHandle = pick.face()
      if (wholeObject) {
        this._commitObject(objectHandle)
      } else {
        this._commit(objectHandle, faceHandle)
      }
    } finally {
      pick.free()
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
    }
  }

  cancel(): void {
    this.hoveredObject = null
    this.hoveredFace = null
    this.lastSnap = null
  }

  private _commit(objectHandle: bigint, faceHandle: bigint): void {
    try {
      this.wasmScene.paint_face(objectHandle, faceHandle, this.currentMaterialId)
      this.onCommit(objectHandle)
    } catch (err) {
      this._handleError(err)
    }
  }

  /** Set the base material on the whole object (⌘/Ctrl-click). */
  private _commitObject(objectHandle: bigint): void {
    try {
      this.wasmScene.set_object_material(objectHandle, this.currentMaterialId)
      this.onCommit(objectHandle)
    } catch (err) {
      this._handleError(err)
    }
  }

  private _handleError(err: unknown): void {
    const code = parseKernelErrorCode(err)
    const rawMsg = err instanceof Error ? err.message : String(err)
    const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
    this.onToast(message, code ?? undefined)
  }
}
