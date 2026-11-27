/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * ShellTool — hollow a solid, leaving chosen faces open.
 *
 * Gesture:
 *   1. Click a face of an object: that face is left open and its object is
 *      the one being hollowed (or select the object first to start closed).
 *      Click an open face again to close it. Clicking another object starts
 *      over with that one.
 *   2. A ghost of the hollowed result shows at once, at a wall thickness that
 *      suits the object. Type a thickness (`2cm`, `3/4"`) to change it; the
 *      ghost follows. A negative thickness puts the wall outside.
 *   3. Enter commits. With no face open the result is a closed hollow body.
 *
 * The kernel owns every rule (`shell_preview` / `shell`); a refusal shows in
 * the status bar while the ghost is up and as a toast on commit.
 */

import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { formatLength, getLengthUnit, parseLengthToMeters, typedReadout } from '../settings/units'
import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { defaultFaceEligible, ineligibleFaceHint } from './faceDraw'
import { buildMeshPreview } from './meshPreview'
import { LineSlot } from './previewLines'
import { cueColors, pathHoverColors } from './FollowMeTool'
import { defaultWallThickness, disposeFill, ringFill, ringSegments, takeTouched, type TouchedIds } from './offsetShared'

export type OnShellCommit = (touched: TouchedIds) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void
export type FaceEligible = (object: bigint, instance: bigint | undefined) => boolean

/** The spec JSON the wasm methods take. Face handles are u64 and can exceed
 *  2^53, so the text is built by hand from decimal strings. */
export function shellSpecJson(open: readonly bigint[], thickness: number): string {
  return `{"open":[${open.map((f) => f.toString()).join(',')}],"thickness":${JSON.stringify(thickness)}}`
}

export class ShellTool implements Tool {
  readonly name = 'Shell'

  private object: bigint | null = null
  private open: bigint[] = []
  private thickness = 0.01
  private buffer = ''
  /** Why the ghost is not showing (the kernel's refusal in plain words). */
  private problem: string | null = null
  private missNotified = false
  private ghost: THREE.Group | null = null
  private readonly hoverLine: LineSlot
  private readonly openLine: LineSlot
  private fills: THREE.Mesh[] = []
  private hoverKey: string | null = null
  private faceEligible: FaceEligible | null = null
  private activeContext: bigint | null = null
  private selection: NodeRef[]
  /** The object a commit just hollowed: the selection the app sends back
   *  after the commit still holds it, and taking it up again would preview
   *  a second shell of the hollow solid (with its refusal in the status
   *  bar). Cleared once the selection is something else. */
  private justShelled: bigint | null = null

  constructor(
    private readonly wasmScene: WasmScene,
    private readonly preview: THREE.Group,
    selection: NodeRef[],
    private readonly onCommit: OnShellCommit,
    private readonly onToast: OnToast,
    private readonly onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.hoverLine = new LineSlot(preview, 995)
    this.openLine = new LineSlot(preview, 996)
    this.selection = selection
    this._adoptSelection()
  }

  /** Replace the face-eligibility rule (the Viewport passes the shared one). */
  setFaceEligibility(pred: FaceEligible | null): void {
    this.faceEligible = pred
  }

  /** Redraw the ghost after the outgoing tool's cancel() has cleared the
   *  shared preview group (ToolController.setTool calls this). */
  activate(): void {
    this._redraw()
  }

  setEditContext(ctx: EditContext): void {
    this.activeContext = ctx.kind === 'object' ? ctx.id : null
  }

  setSelection(nodes: NodeRef[]): void {
    this.selection = nodes
    const only = nodes.length === 1 && nodes[0].kind === 'object' ? nodes[0].id : null
    if (only !== this.justShelled) this.justShelled = null
    if (this.object === null) this._adoptSelection()
  }

  // ------------------------------------------------------------------ state

  private eligible(object: bigint, instance: bigint | undefined): boolean {
    return this.faceEligible !== null
      ? this.faceEligible(object, instance)
      : defaultFaceEligible(this.wasmScene, this.activeContext, object, instance)
  }

  /** A single selected, eligible object starts the tool closed. */
  private _adoptSelection(): void {
    if (this.object !== null) return
    const only = this.selection.length === 1 && this.selection[0].kind === 'object' ? this.selection[0] : null
    if (only === null || only.id === this.justShelled || !this.eligible(only.id, undefined)) return
    this._chooseObject(only.id)
    this._redraw()
    // Taking the selection up is a convenience. When the kernel refuses the
    // closed object (an already hollow one would make the wall pass through
    // itself), do not show that refusal for a click nobody made: stay idle.
    if (this.problem !== null) {
      this._reset()
      this.justShelled = only.id
    }
  }

  /** An undo or redo changed the model: forget which object was just
   *  hollowed, so the selection is judged on what it is now. */
  onHistoryChanged(): void {
    this.justShelled = null
  }

  private _chooseObject(object: bigint): void {
    this.object = object
    this.open = []
    this.buffer = ''
    this.problem = null
    let positions: Float32Array | null = null
    try {
      const mesh = this.wasmScene.object_mesh(object)
      try {
        positions = mesh.positions()
      } finally {
        mesh.free()
      }
    } catch {
      positions = null
    }
    this.thickness = positions === null ? 0.01 : defaultWallThickness(positions)
    this._report()
  }

  private _reset(): void {
    this.object = null
    this.open = []
    this.buffer = ''
    this.problem = null
    this._clearGhost()
    this._clearOpenMarks()
    this.hoverLine.clear()
    this.hoverKey = null
    this.onMeasurement('')
  }

  statusHint(): string {
    if (this.object === null) return 'Click a face to hollow out its object. Faces you click are left open.'
    const wall = `Wall ${formatLength(Math.abs(this.thickness))}${this.thickness < 0 ? ' (outside)' : ''}.`
    const lead = this.problem !== null ? `${this.problem} ` : ''
    return `${lead}${wall} Click faces to leave open. Type a wall thickness and press Enter. A negative thickness adds the wall outside.`
  }

  capturingInput(): boolean {
    return this.buffer !== ''
  }

  hasArmedGesture(): boolean {
    return this.object !== null
  }

  capturesKey(key: string): boolean {
    if (this.object === null) return false
    if (key === ' ') return false // Space always resets to Select
    if (this.buffer === '') return (key >= '0' && key <= '9') || key === '-' || key === '.' || key === 'Enter'
    return key === 'Enter' || isLengthInputKey(key)
  }

  private _report(): void {
    this.onMeasurement(this.buffer !== '' ? typedReadout(this.buffer) : this.object === null ? '' : formatLength(this.thickness))
  }

  // ---------------------------------------------------------------- pointer

  private _pick(ray: Ray): { object: bigint; face: bigint; instance: bigint | undefined } | null {
    const hit = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (hit === undefined) return null
    try {
      return { object: hit.object(), face: hit.face(), instance: hit.instance() }
    } finally {
      hit.free()
    }
  }

  onPointerMove(_snap: Snap | null, ray: Ray): void {
    let hit
    try {
      hit = this._pick(ray)
    } catch {
      hit = null
    }
    if (hit === null || !this.eligible(hit.object, hit.instance)) {
      this.hoverLine.clear()
      this.hoverKey = null
      return
    }
    const key = `${hit.object}:${hit.face}`
    if (key === this.hoverKey) return
    this.hoverKey = key
    this._showHover(hit.object, hit.face)
  }

  private _showHover(object: bigint, face: bigint): void {
    try {
      const ring = this.wasmScene.face_boundary(object, face)
      this.hoverLine.set(ringSegments(ring), pathHoverColors().hover, `${object}:${face}`)
    } catch {
      this.hoverLine.clear()
    }
  }

  onPointerDown(_snap: Snap | null, ray: Ray): void {
    let hit
    try {
      hit = this._pick(ray)
    } catch {
      hit = null
    }
    if (hit === null) {
      if (!this.missNotified) {
        this.missNotified = true
        this.onToast('Click a face of the object you want to hollow out.')
      }
      return
    }
    this.missNotified = false
    if (!this.eligible(hit.object, hit.instance)) {
      this.onToast(ineligibleFaceHint('Shell', this.activeContext !== null, hit.instance))
      return
    }
    if (this.object !== hit.object) this._chooseObject(hit.object)
    const at = this.open.indexOf(hit.face)
    if (at >= 0) this.open.splice(at, 1)
    else this.open.push(hit.face)
    this._redraw()
  }

  // --------------------------------------------------------------- keyboard

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      if (this.buffer !== '') {
        this.buffer = ''
        this._report()
      } else {
        this.cancel()
      }
      return
    }
    if (this.object === null) return
    if (ev.key === 'Enter') {
      this._enter()
      return
    }
    if (!this.capturesKey(ev.key)) return
    this.buffer = editLengthBuffer(this.buffer, ev.key, getLengthUnit())
    this._report()
    const meters = parseLengthToMeters(this.buffer)
    if (meters !== null && meters !== 0) {
      this.thickness = meters
      this._redraw()
    }
  }

  private _enter(): void {
    if (this.buffer !== '') {
      const meters = parseLengthToMeters(this.buffer)
      this.buffer = ''
      if (meters === null || meters === 0) {
        this.onToast('Type a wall thickness, such as 2 cm. A negative one puts the wall outside.')
        this._report()
        return
      }
      this.thickness = meters
    }
    this._commit()
  }

  // ----------------------------------------------------------------- commit

  private _commit(): void {
    if (this.object === null) return
    try {
      const object = this.object
      const change = this.wasmScene.shell(object, shellSpecJson(this.open, this.thickness))
      this._reset()
      this.justShelled = object
      this.onCommit(takeTouched(change))
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      this.onToast(this._explain(code, raw), code ?? undefined)
    }
  }

  /** The refusal in plain words; a too-thick wall names the thickness. */
  private _explain(code: string | null, raw: string): string {
    if (code === 'WallTooThick') return `A ${formatLength(Math.abs(this.thickness))} wall leaves no room here.`
    return kernelErrorMessage(code ?? 'Unknown', raw)
  }

  // ------------------------------------------------------------------ ghost

  private _redraw(): void {
    this._clearGhost()
    this._drawOpenMarks()
    if (this.object === null) return
    let mesh
    try {
      mesh = this.wasmScene.shell_preview(this.object, shellSpecJson(this.open, this.thickness))
      this.problem = null
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      this.problem = this._explain(code, raw)
      return
    }
    const ghost = buildMeshPreview(mesh, cueColors().ok)
    if (ghost === null) return
    this.ghost = ghost
    this.preview.add(ghost)
  }

  private _clearGhost(): void {
    if (this.ghost === null) return
    this.ghost.traverse((o) => {
      if (o instanceof THREE.Mesh || o instanceof THREE.LineSegments) {
        o.geometry.dispose()
        ;(o.material as THREE.Material).dispose()
      }
    })
    this.preview.remove(this.ghost)
    this.ghost = null
  }

  /** The open faces: a fat outline plus a translucent fill each. */
  private _drawOpenMarks(): void {
    this._clearOpenMarks()
    if (this.object === null || this.open.length === 0) return
    const segments: number[] = []
    for (const face of this.open) {
      let ring: Float32Array
      try {
        ring = this.wasmScene.face_boundary(this.object, face)
      } catch {
        continue
      }
      segments.push(...ringSegments(ring))
      const fill = ringFill(ring, pathHoverColors().path)
      if (fill !== null) {
        this.fills.push(fill)
        this.preview.add(fill)
      }
    }
    this.openLine.set(segments, pathHoverColors().path)
  }

  private _clearOpenMarks(): void {
    for (const f of this.fills) {
      this.preview.remove(f)
      disposeFill(f)
    }
    this.fills = []
    this.openLine.clear()
  }

  // ----------------------------------------------------------------- cancel

  cancel(): void {
    this._reset()
  }
}
