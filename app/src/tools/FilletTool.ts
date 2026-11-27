/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * FilletTool — round, bevel, cove or profile edges; resize or remove an
 * existing blend; round a drawn 2D corner.
 *
 * Gesture (docs: true-curves §4.1–4.8, WP10 §3):
 *   - Hover shows what a click would take: an edge (its whole smooth chain),
 *     a face (the edges of the loop nearest the cursor), an existing blend,
 *     a drawn corner, or why nothing can be taken (a smooth edge, or a seam
 *     between two separate objects).
 *   - Click arms it. Moving sizes it live (the kernel's own preview, with
 *     the faces it replaces hidden); click or Enter commits, or type a size
 *     first. A size that does not fit is clamped and the status bar names
 *     what limits it.
 *   - Shift-click collects several picks; typing a size then commits them
 *     all as one step. Double-click repeats the last size.
 *   - After a commit, typing a new size redoes it (the shared retype window).
 *   - Alt (tap) swaps Round and Chamfer. Alt-click on an armed chain adds a
 *     variable-radius handle. End grips set where the blend stops.
 *   - Hovering a blend: click to resize it, Delete to remove it.
 *
 * The kernel owns the truth: previews and commits are `blend_preview` /
 * `blend_edges` and friends; nothing partial reaches it. Typed refusals are
 * toasted with their plain-language copy; while hovering, a refusal shows in
 * the status bar instead.
 */

import * as THREE from 'three'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import { sameRay, type Ray } from '../viewport/math'
import { typedReadout } from '../settings/units'
import { idleRetypeCapturesKey } from './retypeWindow'
import { editCurveBuffer, isCurveInputKey } from './curveInput'
import {
  getFilletSettings,
  profileSizeLabel,
  setFilletMeasurementLabelOverride,
  setFilletThreeWayCorners,
  type FilletProfile,
} from './filletSettings'
import { type V3 } from './filletMath'
import { sameGrip, type GripId, type GripState } from './filletGrips'
import { resolveHover } from './filletPick'
import type { Amount, ArmedStage, ArmTarget, HandleFrame, Stage, FilletHooks, FilletOptions } from './fillet/types'
import { GRIP_DRAG_PX } from './fillet/constants'
import { seamHint, SMOOTH_HINT, BLEND_CHANGED_HINT, gripHoverStatus, gripStatus } from './fillet/hints'
import { tokenText, cornerProfileOk, isDigitStart } from './fillet/helpers'
import { currentAmount, specJson } from './fillet/spec'
import { FilletContext } from './fillet/context'
import { PickStage } from './fillet/pick'
import { SizeStage } from './fillet/size'
import { PreviewStage } from './fillet/preview'
import { CommitStage } from './fillet/commit'
import { EditStageOps } from './fillet/edit'

export type { FilletHooks, FilletOptions } from './fillet/types'
export { FACE_MASK_MAX_TRIANGLES, CLAMPED_RAIL_COLOR, SLOW_PREVIEW_MS, SLOW_PREVIEW_HINT } from './fillet/constants'
export { seamHint } from './fillet/hints'

export class FilletTool implements Tool {
  readonly name = 'Fillet'
  private readonly c: FilletContext
  private altHeld = false
  private pointerDuringAlt = false

  /** The held Alt came with Ctrl or ⌘ (precision snapping), or went to a
   *  text field: not a tap, so its release never swaps the profile. */
  private altChorded = false
  private scheduled = false

  private firstSelection = true
  private activationKeyPending = false

  /** Shift state, pushed by the Viewport's key pair (`setShiftHeld`). */
  private shiftHeld = false
  private _editContext: EditContext = { kind: 'top' }

  constructor(
    wasmScene: WasmScene,
    preview: THREE.Group,
    hooks: FilletHooks,
    opts: FilletOptions = {},
  ) {
    const c = new FilletContext(wasmScene, preview, hooks, opts)
    this.c = c
    c.pick = new PickStage(c)
    c.size = new SizeStage(c)
    c.preview = new PreviewStage(c)
    c.commit = new CommitStage(c)
    c.edit = new EditStageOps(c)
    c.dispatchPointerDown = (snap, ray) => this.onPointerDown(snap, ray)
    setFilletThreeWayCorners(0)
  }

  statusHint(): string {
    const chamfer = getFilletSettings().profile.kind === 'chamfer'
    const other = chamfer ? 'round' : 'chamfer'
    const verb = chamfer ? 'chamfer' : 'round'
    const size = chamfer ? 'a distance' : 'a radius'
    if (this.c.statusLine !== null) return this.c.statusLine
    if (this.c.note !== null) return this.c.note
    const s = this.c.stage
    if (s.kind === 'armed') {
      if (s.grab !== null) return gripStatus(s.grab)
      if (s.gripHover !== null) return gripHoverStatus(s.gripHover)
      if (s.active.kind !== 'radius') return `${gripHoverStatus(s.active).replace('Click or drag this grip to', 'Type to')} Tab for the next field.`
      return `Drag the handle or type ${size}. Click the handle or press Enter to finish.`
    }
    if (s.kind === 'edit') {
      if (s.grab) return 'Move to resize it, then let go. Or type a size.'
      return 'Drag the handle or type a size. Click to finish. Delete removes the fillet.'
    }
    if (this.c.collected.length > 0) {
      const n = this.c.collected.reduce((sum, c) => sum + c.chain.edges.length, 0)
      return `${n} edge${n === 1 ? '' : 's'} picked. Shift-click to add more, click one to size them, or type ${size}.`
    }
    // A hovered edge or corner takes a typed size itself (`typingPicksHover`),
    // so the hint names it rather than the retype.
    const hoverTakesTyping =
      this.c.retypeTyped === '' &&
      this.c.collected.length === 0 &&
      (this.c.hover.kind === 'edge' || (this.c.hover.kind === 'corner' && cornerProfileOk()))
    // A blend under the pointer says what a click or Delete does to it,
    // whether or not the last fillet is still retypeable.
    // Only a click or Delete checks whether a later edit changed it.
    if (this.c.hover.kind === 'blend') {
      return this.c.blendKnownChanged(this.c.hover.object, this.c.hover.blendId)
        ? BLEND_CHANGED_HINT
        : 'Click to resize this fillet. Delete to remove it.'
    }
    if (this.c.retype.isOpen && !hoverTakesTyping) {
      // A round that varied (stops or radius points) repeats as its base
      // size only: a double-click carries no stops or radius points.
      if (this.c.lastVaried && this.c.lastToken !== null) {
        return `Type ${size} to adjust. Double-click another edge to repeat ${tokenText(this.c.lastToken)} as a plain ${verb}.`
      }
      const repeat = this.c.lastToken === null ? '' : ` ${tokenText(this.c.lastToken)}`
      return `Type ${size} to adjust. Double-click another edge to repeat${repeat}.`
    }
    switch (this.c.hover.kind) {
      case 'corner':
        return `Click to round this corner, or type ${size}.`
      case 'seam':
        return seamHint(this.c.hover.names, this.c.hover.group)
      case 'smooth':
        return SMOOTH_HINT
      default:
        return `Click an edge or a face to ${verb} it. Shift-click to pick several. Alt = ${other}.`
    }
  }

  capturingInput(): boolean {
    return this.c.stage.kind !== 'idle' || (this.c.retype.isOpen && this.c.retypeTyped !== '')
  }

  hasArmedGesture(): boolean {
    return this.c.stage.kind !== 'idle' || this.c.collected.length > 0 || this.c.retypeTyped !== ''
  }

  /** True while a text field has focus: keys belong to it, not the tool. */
  private typingElsewhere(): boolean {
    if (typeof document === 'undefined') return false
    const a = document.activeElement as HTMLElement | null
    return a !== null && (a.tagName === 'INPUT' || a.tagName === 'TEXTAREA' || a.isContentEditable)
  }

  capturesKey(key: string): boolean {
    const idle = this.c.stage.kind === 'idle'
    if (this.typingElsewhere()) return false
    if (!idle) return this.stageCapturesKey(key)
    if (this.c.collected.length > 0 && (isDigitStart(key) || this.c.retypeTyped !== '')) return true
    if (this.c.hover.kind === 'blend' && (key === 'Delete' || key === 'Backspace')) return true
    if (this.typingPicksHover(key)) return true
    if (this.c.retype.isOpen) return idleRetypeCapturesKey(key, this.c.retypeTyped, isCurveInputKey)
    return false
  }

  /** A size typed over a pickable edge, face or corner picks it, as a click
   *  would: the first digit arms the hovered target and starts its size.
   *  A hovered edge or corner (small, deliberate targets) wins over
   *  retyping the last fillet; a face, which the pointer drifts onto
   *  easily, picks only when there is no fillet to retype. */
  private typingPicksHover(key: string): boolean {
    if (!isDigitStart(key) || this.c.retypeTyped !== '' || this.c.collected.length > 0) return false
    const k = this.c.hover.kind
    if (k === 'corner') return cornerProfileOk()
    return k === 'edge' || (k === 'face' && !this.c.retype.isOpen)
  }

  /** The keys an armed or editing stage owns: its own controls, and what
   *  can start or continue a typed size. Anything else (Space, a tool's
   *  letter) goes to the app, whose tool switch cancels this stage, so the
   *  keyboard never stays locked into Fillet. Delete and Backspace stay
   *  here so they cannot delete the selection mid-gesture. */
  private stageCapturesKey(key: string): boolean {
    if (key === 'Escape' || key === 'Enter' || key === 'Backspace' || key === 'Delete' || key === 'Tab') return true
    if (key === 'Alt' || key === 'Shift' || key === 'Control' || key === 'Meta') return true
    // Shift+F again mid-gesture is this tool's own shortcut: swallow it
    // rather than restart the tool and drop the pick.
    if (key === 'F') return true
    if (key === ' ') return false
    const s = this.c.stage
    // An active grip types into its own buffer; the capture follows it.
    const typed = s.kind === 'armed' && s.active.kind !== 'radius' ? s.gripTyped : s.kind === 'armed' || s.kind === 'edit' ? s.typed : ''
    if (typed === '') return (key >= '0' && key <= '9') || key === '-' || key === '.'
    return isCurveInputKey(key)
  }

  /** Quietly close the retype window — the host calls this before an
   *  explicit undo/redo/delete. */
  disarmRetype(): void {
    this.c.retype.close()
    this.c.retypeTyped = ''
    if (this.c.stage.kind === 'idle') this.c.measure('')
  }

  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
    this.cancel()
  }

  setSelection(nodes: Array<{ kind: string; id: bigint }>): void {
    // The first push after activation arms the selected objects' sharp edges
    // (A6); later pushes (outliner clicks, undo pruning) must not.
    if (!this.firstSelection) return
    this.firstSelection = false
    if (this.c.stage.kind !== 'idle') return
    const objects = nodes.filter((n) => n.kind === 'object').map((n) => n.id)
    if (objects.length === 0) return
    this.c.pick.armObjects(objects)
    // The Shift+F that activated the tool can still be on its way to onKey
    // (the Viewport forwards keys to an armed, capturing tool); it must not
    // land in the radius buffer as an `F`.
    this.activationKeyPending = (this.c.stage as Stage).kind === 'armed'
  }

  /** Cmd/Ctrl+A over an object: arm every sharp edge of the object under the
   *  cursor, and only that one. While an earlier arming is showing (for
   *  example the one made from the selection on activation) the hover is not
   *  tracked, so it is re-read at the last pointer ray and the earlier
   *  arming is replaced. False (nothing done) unless a solid is under the
   *  cursor. */
  armHoveredObject(): boolean {
    let h = this.c.hover
    if (this.c.stage.kind === 'armed') {
      if (this.c.lastRay === null) return false
      h = resolveHover(this.c.wasmScene, this.c.lastRay, this.c.pickOptions())
    } else if (this.c.stage.kind !== 'idle') {
      return false
    }
    if (h.kind !== 'edge' && h.kind !== 'face' && h.kind !== 'blend') return false
    if (this.c.stage.kind === 'armed') this.resetStage()
    this.c.collected = []
    this.c.pick.armObjects([h.object])
    return (this.c.stage as Stage).kind === 'armed'
  }

  updateGripScale(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    this.c.worldPerPixel = worldPerPixel
    this.c.gripsView.updateScale(camera, worldPerPixel)
    this.c.labels.update(camera, worldPerPixel)
  }

  onPointerScreenMove(x: number, y: number): void {
    this.c.screen = [x, y]
  }

  /** Alt held / released, from the Viewport's Alt key pair (I-26). A tap
   *  (no pointer press while held) swaps Round and Chamfer; Alt plus a click
   *  is a click, not a toggle. `chorded` marks an Alt that is not a bare
   *  tap (Ctrl or ⌘ down with it, or a text field had focus): the Ctrl+Alt
   *  precision-snap chord must not flip the profile on release. */
  setAltHeld(held: boolean, chorded = false): void {
    if (held) {
      if (this.altHeld) return
      this.altHeld = true
      this.pointerDuringAlt = false
      this.altChorded = chorded
      return
    }
    if (!this.altHeld) return
    this.altHeld = false
    if (!this.pointerDuringAlt && !this.altChorded && !chorded) this.c.edit.toggleKind()
    this.pointerDuringAlt = false
    this.altChorded = false
  }

  /** Ctrl or ⌘ pressed while Alt is already held: the hold became a chord. */
  markAltChorded(): void {
    if (this.altHeld) this.altChorded = true
  }

  /** The window lost focus mid-hold: forget the hold without toggling. */
  resetAlt(): void {
    this.altHeld = false
    this.pointerDuringAlt = false
    this.altChorded = false
  }

  get isAltHeld(): boolean {
    return this.altHeld
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    // Activated from the rail or a menu: no activation key is coming.
    this.activationKeyPending = false
    if (!sameRay(this.c.noteRay, ray)) this.c.note = null
    this.c.lastSnap = snap
    this.c.pendingMove = { ray, snap }
    if (this.scheduled) return
    this.scheduled = true
    this.c.schedule(() => {
      this.scheduled = false
      this.flushMove()
    })
  }

  /** Runs the coalesced pointer move now (tests, and before a click). */
  flushMove(): void {
    const m = this.c.pendingMove
    this.c.pendingMove = null
    if (m === null) return
    this.handleMove(m.ray)
    this.c.hooks.requestRender()
  }

  /** Puts the preview root back in the shared group after a `cancel()`. */
  private attach(): void {
    if (this.c.root.parent !== this.c.previewGroup) this.c.previewGroup.add(this.c.root)
  }

  private handleMove(ray: Ray): void {
    this.attach()
    this.c.lastRay = ray
    const s = this.c.stage
    if (s.kind === 'armed') {
      if (s.grab !== null) {
        // A length being typed for the grip pins it; the pointer no longer
        // moves it or overwrites the typed readout.
        if (s.grab.kind === 'radius') {
          if (s.typed === '') this.c.size.sizeFromHandle(s, ray)
        } else if (s.gripTyped === '') {
          this.c.size.dragGrip(ray)
        }
        return
      }
      // Nothing held: the pointer sizes nothing; only the grip under it
      // changes (its hint and highlight).
      const under = this.c.size.gripUnder(s, ray)
      if (!sameGrip(under, s.gripHover)) {
        s.gripHover = under
        this.c.size.refreshGripMarkers()
      }
      return
    }
    if (s.kind === 'edit') {
      if (s.grab && s.typed === '') {
        this.c.edit.editSizeFromHandle(s, ray)
        this.c.edit.drawEditPreview()
      }
      return
    }
    this.c.pick.updateHover(ray)
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    this.c.note = null
    // Activated from the rail or a menu: no activation key is coming.
    this.activationKeyPending = false
    this.c.lastSnap = snap
    this.flushMove()
    this.attach()
    this.c.retype.close()
    this.c.retypeTyped = ''
    if (this.altHeld) this.pointerDuringAlt = true
    this.c.lastRay = ray
    this.c.finishedAt = null
    const s = this.c.stage

    if (s.kind === 'armed') {
      this.c.size.pointerDownArmed(s, snap, ray)
      return
    }
    if (s.kind === 'edit') {
      this.c.edit.pointerDownEdit(s, snap, ray)
      return
    }
    // Idle.
    const hover = resolveHover(this.c.wasmScene, ray, this.c.pickOptions())
    this.c.hover = hover
    if (this.isShiftClick()) {
      this.c.pick.toggleCollected(hover)
      return
    }
    switch (hover.kind) {
      case 'edge':
      case 'face':
        this.c.pick.armFromHover(hover, true)
        break
      case 'corner':
        this.c.pick.armCorner(hover.corner, true)
        break
      case 'blend':
        this.c.edit.enterEdit(hover, ray, true)
        break
      case 'seam':
        this.c.hooks.onToast(seamHint(hover.names, hover.group))
        break
      case 'smooth':
        this.c.hooks.onToast(SMOOTH_HINT)
        break
      default:
        break
    }
  }

  /** The first click of a double armed the edge at the last size (or
   *  entered an edit); the double commits it as it stands. */
  onDoubleClick(_snap: Snap | null, _ray: Ray): boolean {
    const s = this.c.stage
    if (s.kind === 'armed') {
      if (currentAmount(s) === null) return false
      this.c.commit.commitArmed(this.c.size.armedCommitAmount(s))
      return true
    }
    if (s.kind === 'edit') {
      this.c.edit.commitEdit()
      return true
    }
    // The first click of this double already finished a pick (a click on
    // the handle, on a band): the double is spent, not a context change.
    if (this.c.finishedAt !== null && !this.c.movedEnough(this.c.finishedAt)) {
      this.c.finishedAt = null
      return true
    }
    return false
  }

  private isShiftClick(): boolean {
    return this.shiftHeld
  }

  setShiftHeld(held: boolean): void {
    this.shiftHeld = held
  }

  onKey(ev: KeyboardEvent): void {
    if (this.typingElsewhere()) return
    const key = ev.key
    if (key === 'Alt' || key === 'Shift' || key === 'Control' || key === 'Meta') return
    this.c.note = null
    const pending = this.activationKeyPending
    this.activationKeyPending = false
    if (pending && (key === 'F' || key === 'f')) return
    const s = this.c.stage
    if (s.kind === 'armed' || s.kind === 'edit') {
      // A key the stage turned away (it went to the app) must not land in
      // the typed size either; nor does a repeated Shift+F.
      const buffer = s.kind === 'armed' && s.active.kind !== 'radius' ? s.gripTyped : s.typed
      if (key === 'F' && buffer === '') return
      if (!this.stageCapturesKey(key)) return
    }

    if (key === 'Escape') {
      this.escape()
      return
    }

    if (s.kind === 'armed') {
      if (key === 'Enter') {
        this.c.size.enterOnArmed(s)
        return
      }
      if (key === 'Tab') {
        this.c.size.cycleActive(s)
        return
      }
      if (key === 'Delete' && s.gripTyped === '' && this.c.size.deleteActivePoint(s)) return
      if (s.active.kind !== 'radius') {
        // A length for the active grip; the blend's own size is untouched.
        if (isCurveInputKey(key)) {
          s.gripTyped = editCurveBuffer(s.gripTyped, key)
          this.c.measure(s.gripTyped === '' ? this.c.size.gripReadout(s) : typedReadout(s.gripTyped))
          this.c.size.refreshGripMarkers()
        }
        return
      }
      if (isCurveInputKey(key)) {
        s.typed = editCurveBuffer(s.typed, key)
        this.c.measure(s.typed === '' ? '' : typedReadout(s.typed))
        this.c.size.applyTypedToPreview(s)
      }
      return
    }

    if (s.kind === 'edit') {
      if (key === 'Delete' || key === 'Backspace') {
        if (s.typed === '') {
          this.c.edit.removeEdited()
          return
        }
      }
      if (key === 'Enter') {
        this.c.edit.enterOnEdit(s)
        return
      }
      if (isCurveInputKey(key)) {
        s.typed = editCurveBuffer(s.typed, key)
        this.c.measure(s.typed === '' ? '' : typedReadout(s.typed))
        this.c.edit.drawEditPreview()
      }
      return
    }

    // Idle.
    if (this.c.hover.kind === 'blend' && (key === 'Delete' || key === 'Backspace')) {
      this.c.edit.removeHovered(this.c.hover)
      return
    }
    if (this.c.collected.length > 0 && (isDigitStart(key) || this.c.retypeTyped !== '')) {
      this.c.pick.typeIntoCollected(key)
      return
    }
    if (this.typingPicksHover(key)) {
      // Resolve again, as a click does: the scene may have changed (an undo,
      // a delete) since the pointer last moved.
      if (this.c.lastRay !== null) this.c.hover = resolveHover(this.c.wasmScene, this.c.lastRay, this.c.pickOptions())
      const h = this.c.hover
      if (this.typingPicksHover(key)) {
        if (h.kind === 'corner') this.c.pick.armCorner(h.corner)
        else if (h.kind === 'edge' || h.kind === 'face') this.c.pick.armFromHover(h)
      }
      if (this.c.stage.kind === 'armed') {
        // Armed: the digit is the size's first character.
        this.c.retype.close()
        this.onKey(ev)
        return
      }
      // Nothing armed: the digit retypes the last fillet, as before.
      this.c.pick.redrawHover()
    }
    if (this.c.retype.isOpen && idleRetypeCapturesKey(key, this.c.retypeTyped, isCurveInputKey)) {
      this.c.commit.retypeKey(key)
    }
  }

  /** What the measurement box calls its value: the profile's size name, or
   *  the held grip's ("Stop", "Position", "Radius"). */
  measurementLabel(): string {
    return this.c.heldGripLabel() ?? profileSizeLabel(getFilletSettings().profile)
  }

  private escape(): void {
    const s = this.c.stage
    if (s.kind !== 'idle') {
      this.resetStage()
      return
    }
    if (this.c.retype.isOpen || this.c.retypeTyped !== '') {
      this.disarmRetype()
      return
    }
    if (this.c.collected.length > 0) {
      this.c.collected = []
      this.c.pick.redrawHover()
      return
    }
    // Nothing left to cancel: Esc stays in Fillet. Space (handled by the
    // app) is the way to Select.
  }

  cancel(): void {
    setFilletMeasurementLabelOverride(null)
    this.c.note = null
    // A move still queued would hover again on a tool that is gone.
    this.c.pendingMove = null
    this.resetStage()
    this.c.previewGroup.remove(this.c.root)
    this.c.collected = []
    this.c.retype.close()
    this.c.retypeTyped = ''
    this.c.hover = { kind: 'none' }
    this.c.statusLine = null
    this.c.dropBlendNotes()
    this.c.preview.clearHighlight()
    this.c.measure('')
    this.c.pick.syncCornerProfile()
  }

  /** Back to idle with the collected picks kept (the first Esc). */
  private resetStage(): void {
    this.c.unsubSettings?.()
    this.c.unsubSettings = null
    this.c.stage = { kind: 'idle' }
    this.c.preview.clearPreviewDrawing()
    this.c.gripsView.clear()
    this.c.labels.clear()
    this.c.statusLine = null
    setFilletThreeWayCorners(0)
    this.c.measure('')
    this.c.pick.redrawHover()
  }

  setCtrlHeld(held: boolean): void {
    this.c.ctrlHeld = held
  }

  /** A grip pressed, moved and released is placed where it was let go; a
   *  press and release in place keeps it picked up for a move and a click. */
  onPointerUp(_snap: Snap | null, ray: Ray): void {
    const s = this.c.stage
    if (s.kind === 'edit') {
      if (s.grab) {
        this.c.edit.releaseEditHandle(s, ray)
        this.c.hooks.requestRender()
      }
      return
    }
    if (s.kind !== 'armed' || s.grab === null) return
    if (s.grab.kind === 'radius') {
      this.c.size.releaseHandle(s, ray)
      this.c.hooks.requestRender()
      return
    }
    // A length being typed for the grip pins it: the release neither moves
    // it nor drops what was typed (Enter applies it).
    if (s.gripTyped !== '') return
    // A short drag counts (a radius grip moved a few millimeters sits only a
    // few pixels from where it was picked up); only a press and release in
    // place is a click that keeps the grip picked up.
    const moved = s.grabScreen === null ? !sameRay(s.grabRay, ray) : this.c.movedEnough(s.grabScreen, GRIP_DRAG_PX)
    if (!moved) return
    this.c.pendingMove = null
    this.c.size.dragGrip(ray)
    this.c.size.putGripDown(s)
    this.c.hooks.requestRender()
  }

  /** The radius handle's frame while armed or editing (world), for tests
   *  and the E2E harness. */
  gizmoHandle(): HandleFrame | null {
    const s = this.c.stage
    if (s.kind === 'armed') return s.handle
    if (s.kind === 'edit') return s.handle
    return null
  }

  /** Where the gizmo marker `id` is (world), or null when it is not shown. */
  gizmoMarker(id: GripId): V3 | null {
    return this.c.gripsView.marker(id)?.pos ?? null
  }

  // Seams the kernel tests reach through a cast: each forwards to the stage
  // or module that owns the logic.

  private get stage(): Stage {
    return this.c.stage
  }

  private armedCommitAmount(s: ArmedStage): Amount {
    return this.c.size.armedCommitAmount(s)
  }

  private commitTargets(targets: ArmTarget[], profile: FilletProfile, amount: Amount, grips?: GripState): void {
    this.c.commit.commitTargets(targets, profile, amount, grips)
  }

  private specJson(...args: Parameters<typeof specJson>): string {
    return specJson(...args)
  }

  private refusalText(err: unknown): string {
    return this.c.preview.refusalText(err)
  }

  onDocumentReset(): void {
    this.cancel()
  }

  onHistoryChanged(): void {
    this.c.dropBlendNotes()
    this.c.retype.close()
    this.c.retypeTyped = ''
    if (this.c.stage.kind !== 'idle') this.resetStage()
    this.c.collected = []
  }

  /** Releases scene objects the tool added. */
  dispose(): void {
    this.cancel()
    this.c.gripsView.dispose()
    this.c.labels.dispose()
    this.c.previewGroup.remove(this.c.root)
  }
}
