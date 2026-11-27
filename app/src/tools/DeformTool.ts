/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * DeformTool — Twist, Bend, Taper and Bend to Path (true-curves.md §5.3):
 * reshape a whole solid with a smooth map instead of moving its faces.
 *
 * Twist, Bend and Taper:
 *   1. The solid is the selection (or click one). Its bounding box appears
 *      with a grip on each face.
 *   2. Click a face of the box. That face is the moving end; the opposite
 *      face stays put.
 *   3. Move: Twist follows the angle the pointer sweeps around the axis,
 *      Bend follows how far it moves sideways, Taper follows how far it is
 *      from the axis compared with where you grabbed. A ghost shows the
 *      kernel's own result each frame. If the result would pass through
 *      itself, the ghost stops at the last amount that works.
 *   4. Click to finish, or type the amount: `25°` or `25` for an angle,
 *      `60%` for a taper, `8s` for the number of steps along the length.
 *   After the commit, a typed amount redoes the deform.
 *
 * Bend to Path:
 *   1. The solid is the selection.
 *   2. Pick the base line the solid lies along: an edge, or two points.
 *   3. Pick the path: click a drawn line or curve.
 *   4. A ghost shows the result; click or Enter finishes.
 *
 * The kernel owns every decision (solid, amount range, self-intersection);
 * this tool gathers the frame and the amount and shows its refusals with the
 * shared kernel-error copy.
 */

import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import { sameRay, type Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'
import { getLengthUnit } from '../settings/units'
import { rayPlaneIntersect, subV3, normalizeV3 } from '../viewport/geoHelpers'
import { BoxGizmo, gripsFromBox, type Grip } from './boxGizmo'
import { LineSlot } from './previewLines'
import { buildMeshPreview } from './meshPreview'
import { cueColors, pathHoverColors } from './FollowMeTool'
import { pickEdgePath, edgePathPolyline, loopPolyline, polylineSegments, type PolylinePath } from './edgePathPick'
import { pickFaceAt } from './profilePick'
import { RetypeWindow, idleRetypeCapturesKey, retypeStaleMessage } from './retypeWindow'
import {
  editShapeBuffer,
  isBareModifier,
  isShapeInputKey,
  parseAngleBuffer,
  parseRatioBuffer,
  parseShapeToken,
  shapeReadout,
  formatDegrees,
  formatPercent,
} from './shapeInput'
import {
  FOUR_PI,
  TWO_PI,
  TwistTracker,
  angleAround,
  bendAngle,
  bendCrossAxis,
  boxSpecJson,
  clampToLimit,
  frameForFace,
  hitMovingPlane,
  identityAmount,
  movingCenter,
  pathSpecJson,
  radiusFromAxis,
  rayBoxFace,
  taperRatio,
  boxFaces,
  type BoxFace,
  type BoxMode,
  type DeformFrame,
  type V3,
} from './deformMath'
import { axisFromPoints } from './revolveMath'
import { getDeformMode, DEFORM_MODE_LABEL, type DeformMode } from './deformMode'
import type { NodeRef } from '../panels/treeModel'
import { i18n } from '@lingui/core'
import { BUSY_REFUSED } from '../busyCard'

/** Bend to Path's refusal of a drawn path that branches. */
const PATH_FORKS =
  'The path forks. Bend to Path follows one run of lines with no branches: erase the extra line, or draw the path on its own.'

export type OnDeformCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

const GHOST_COLOR = 0xee8800
const AMOUNT_EPS = 1e-5
/** Pointer travel (as a fraction of the box length) before Bend commits to a side. */
const BEND_DECIDE_FRACTION = 0.03

/** The opening of the status while a drag on an object too heavy to
 *  preview runs without a ghost (the kernel's preview gave up at its work
 *  cap); the drag's own guidance follows it. */
export const DEFORM_PREVIEW_SKIPPED_HINT = 'Preview skipped: this object is too large to preview while you move.'

/** The drag stage's guidance per mode. */
const DRAG_HINT: Record<BoxMode, string> = {
  twist: 'Move to twist, click to finish, or type an angle (25°). Esc backs up.',
  bend: 'Move sideways to bend, click to finish, or type an angle (25°). Esc backs up.',
  taper: 'Move to taper, click to finish, or type a percentage (60%). Esc backs up.',
}

/** The toast when a commit could not start because another long change is
 *  still running. */
export const DEFORM_BUSY_REFUSED = BUSY_REFUSED

/** The busy card's line while a deform commits. */
const DEFORM_BUSY: Record<DeformMode, string> = {
  twist: 'Twisting…',
  bend: 'Bending…',
  taper: 'Tapering…',
  'bend-to-path': 'Bending…',
}

interface Box {
  min: V3
  max: V3
}

interface DragState {
  face: BoxFace
  frame: DeformFrame
  /** Where the click landed on the moving face. */
  grab: V3
  /** The plane Bend measures the pointer on (through `grab`, facing the eye). */
  dragNormal: V3
  tracker: TwistTracker
  grabRadius: number | null
  /** Bend: the cross axis, once the drag has chosen a side. */
  bendU: V3 | null
  amount: number
  /** The kernel's limit per side of the identity, fetched at most once each. */
  limits: { neg: number | null; pos: number | null }
  limited: boolean
}

type Stage =
  | { kind: 'pick-object' }
  | { kind: 'pick-face'; object: bigint; box: Box }
  | { kind: 'drag'; object: bigint; box: Box; drag: DragState }
  | { kind: 'pick-base'; object: bigint; first: V3 | null }
  | { kind: 'pick-path'; object: bigint; base: { from: V3; to: V3 } }
  | { kind: 'path-ready'; object: bigint; base: { from: V3; to: V3 }; path: PolylinePath }

/** What a retype redoes: the deform with a new amount. */
interface RetypeSpec {
  object: bigint
  mode: BoxMode
  frame: DeformFrame
  amount: number
  stations: number
}

export class DeformTool implements Tool {
  readonly name = 'Deform'

  private mode: DeformMode
  private stage: Stage = { kind: 'pick-object' }
  private selection: NodeRef[]
  private acquire: ((ray: Ray) => NodeRef[] | null) | null = null

  private readonly gizmo = new BoxGizmo()
  private grips: Grip[] | null = null
  private readonly hoverLine: LineSlot
  private readonly pathLine: LineSlot
  private readonly axisLine: LineSlot
  private ghost: THREE.Group | null = null
  private ghostKey = ''

  private typed = ''
  private stations = 0
  private problem = ''
  private limitNote = ''
  /** A one-line confirmation ("Stations set to 8.") shown until the next
   *  pointer move or key. `noteRay` is the ray it was set under: the
   *  Viewport re-sends that ray after a key, which is not a move. */
  private note = ''
  private noteRay: Ray | null = null
  private lastRay: Ray | null = null
  private missNotified = false
  /** Objects whose preview gave up for size (`PreviewTooLarge`): their
   *  drags run without a ghost or a clamp, and their commits go behind the
   *  busy card. */
  private readonly tooLarge = new Set<bigint>()
  /** The host's busy card: shows a line, lets it paint, then runs `work`;
   *  `false` when it cannot take the work (another card is up). */
  private busy: ((message: string, work: () => void) => boolean | void) | null = null
  /** The document generation `tooLarge` was learned at: any change since
   *  (an undo, an edit from anywhere) forgets it. */
  private skipGeneration: bigint | null = null
  private readonly retype: RetypeWindow<RetypeSpec>
  private _editContext: EditContext = { kind: 'top' }

  constructor(
    private readonly wasmScene: WasmScene,
    private readonly preview: THREE.Group,
    private readonly objectsGroup: THREE.Group | null,
    initialSelection: readonly NodeRef[],
    private readonly onCommit: OnDeformCommit,
    private readonly onToast: OnToast,
    private readonly onMeasurementCb: OnMeasurement = () => { /* no-op */ },
    mode: DeformMode = getDeformMode(),
  ) {
    this.mode = mode
    this.selection = [...initialSelection]
    this.retype = new RetypeWindow(wasmScene)
    this.hoverLine = new LineSlot(preview, 995)
    this.pathLine = new LineSlot(preview, 996)
    this.axisLine = new LineSlot(preview, 997)
    this._restart()
  }

  // ------------------------------------------------------------- Tool plumbing

  /** Host hook: auto-select what a click lands on when nothing is selected. */
  setSelectionAcquirer(fn: ((ray: Ray) => NodeRef[] | null) | null): void {
    this.acquire = fn
  }

  /** Host hook: the busy card a commit on an object too heavy to preview
   *  runs behind (it can take seconds). Without it the commit runs at once. */
  setBusyRunner(fn: ((message: string, work: () => void) => boolean | void) | null): void {
    this.busy = fn
  }

  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
    this.cancel()
  }

  /** Keep the tool in step with the app selection while idle. */
  setSelection(nodes: NodeRef[]): void {
    this.selection = [...nodes]
    if (this.stage.kind === 'pick-object' || this.stage.kind === 'pick-face') this._restart()
  }

  /** The Twist / Bend / Taper / Bend to Path menu entries while the tool is
   *  already running: restart in the new mode, keeping the solid. */
  setMode(mode: DeformMode): void {
    if (mode === this.mode) return
    this.mode = mode
    this._resetGesture()
    this._restart()
  }

  activate(): void {
    if (this.stage.kind === 'pick-face') this._showGizmo()
  }

  capturingInput(): boolean {
    return this.stage.kind !== 'pick-object' && this.stage.kind !== 'pick-face'
  }

  hasArmedGesture(): boolean {
    return this.capturingInput() || this.typed !== ''
  }

  capturesKey(key: string): boolean {
    if (this.stage.kind === 'drag' || this.stage.kind === 'path-ready') {
      return key === 'Enter' || (this.stage.kind === 'drag' && isShapeInputKey(key))
    }
    if (this.stage.kind === 'pick-face' && this.retype.isOpen) {
      return idleRetypeCapturesKey(key, this.typed, isShapeInputKey)
    }
    return false
  }

  disarmRetype(): void {
    this.retype.close()
    if (this.typed !== '' && this.stage.kind === 'pick-face') {
      this.typed = ''
      this.onMeasurementCb('')
    }
  }

  onHistoryChanged(): void {
    this.retype.close()
    // Undo, redo or another edit may have changed any object's size.
    this.tooLarge.clear()
  }

  onDocumentReset(): void {
    this.selection = []
    this.tooLarge.clear()
    this.cancel()
  }

  updateGripScale(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    this.gizmo.updateScale(camera, worldPerPixel)
  }

  statusHint(): string {
    // A limit or a problem outranks the confirmation note.
    if (this.limitNote !== '') return this.limitNote
    if (this.problem !== '') return this.problem
    if (this.note !== '') return this.note
    const verb = DEFORM_MODE_LABEL[this.mode].toLowerCase()
    switch (this.stage.kind) {
      case 'pick-object':
        return `Click a solid to ${verb}.`
      case 'pick-face':
        if (this.retype.isOpen) {
          return this.mode === 'taper'
            ? 'Type a percentage (60%) to redo the taper, or click a face to start another.'
            : 'Type an angle (25°) to redo it, or click a face to start another.'
        }
        return this.mode === 'bend'
          ? 'Click the end of the box to bend. The opposite end stays put.'
          : `Click the end of the box to ${verb}. The opposite end stays put.`
      case 'drag':
        if (this.tooLarge.has(this.stage.object)) {
          return `${DEFORM_PREVIEW_SKIPPED_HINT} ${DRAG_HINT[this.mode as BoxMode]}`
        }
        return DRAG_HINT[this.mode as BoxMode]
      case 'pick-base':
        return this.stage.first === null
          ? 'Click an edge or two points for the line the solid bends from.'
          : 'Click the second point of the line.'
      case 'pick-path':
        return 'Click the path to bend the solid along — a drawn line or curve.'
      case 'path-ready':
        return 'Click or press Enter to bend the solid along the path. Esc backs up.'
    }
  }

  cancel(): void {
    this.note = ''
    this._resetGesture()
    this.retype.close()
    this.stage = { kind: 'pick-object' }
    this.missNotified = false
    this.gizmo.forget()
    this.grips = null
    this.hoverLine.clear()
    this.pathLine.clear()
    this.axisLine.clear()
    this.ghost = null
    this.ghostKey = ''
    this.onMeasurementCb('')
    clearPreview(this.preview)
  }

  // --------------------------------------------------------------- starting up

  /** The one object the selection names, or null. */
  private _selectedObject(): bigint | null {
    if (this.selection.length !== 1) return null
    const n = this.selection[0]
    return n.kind === 'object' ? n.id : null
  }

  /** Put the tool at its first stage for the current selection and mode. */
  private _restart(): void {
    this._hideGizmo()
    this.hoverLine.clear()
    const object = this._selectedObject()
    if (object === null) {
      this.stage = { kind: 'pick-object' }
      return
    }
    if (this.mode === 'bend-to-path') {
      this.stage = { kind: 'pick-base', object, first: null }
      return
    }
    const box = this._boxOf(object)
    if (box === null) {
      this.stage = { kind: 'pick-object' }
      return
    }
    this.stage = { kind: 'pick-face', object, box }
    this._showGizmo()
  }

  private _resetGesture(): void {
    this.typed = ''
    this.stations = 0
    this.problem = ''
    this.limitNote = ''
    this._clearGhost()
    this.axisLine.clear()
    this.pathLine.clear()
    this.onMeasurementCb('')
  }

  /** The world bounding box of one object, from its rendered group. */
  private _boxOf(object: bigint): Box | null {
    const group = this.objectsGroup?.getObjectByName(`Object_${object}`)
    if (group === undefined) return null
    const box = new THREE.Box3().expandByObject(group)
    if (box.isEmpty()) return null
    return { min: [box.min.x, box.min.y, box.min.z], max: [box.max.x, box.max.y, box.max.z] }
  }

  private _showGizmo(): void {
    this._hideGizmo()
    if (this.stage.kind !== 'pick-face') return
    const { min, max } = this.stage.box
    // Only the six face grips: Deform drives one end face at a time.
    this.grips = gripsFromBox(min, max, false).filter((g) => g.axisIndex !== null && g.axisIndex2 === null)
    this.gizmo.draw(this.preview, min, max, this.grips, 'DeformGizmo')
  }

  private _hideGizmo(): void {
    this.gizmo.hide(this.preview)
    this.grips = null
  }

  // ------------------------------------------------------------------ pointer

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (!sameRay(this.noteRay, ray)) this.note = ''
    this.lastRay = ray
    switch (this.stage.kind) {
      case 'pick-object':
        return
      case 'pick-face':
        this._hoverFace(ray)
        return
      case 'drag':
        this._dragTo(ray)
        return
      case 'pick-base':
        this._hoverBase(snap)
        return
      case 'pick-path':
        this._hoverPath(ray)
        return
      case 'path-ready':
        return
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    this.note = ''
    this.disarmRetype()
    switch (this.stage.kind) {
      case 'pick-object':
        this._acquireObject(ray)
        return
      case 'pick-face': {
        const face = this._faceAt(ray)
        if (face !== null) this._beginDrag(face, ray)
        return
      }
      case 'drag':
        this._dragTo(ray)
        // The click commits the pointer's amount; a buffer typed meanwhile
        // survives a card that cannot take the commit.
        this._commitDrag(undefined, this.typed)
        return
      case 'pick-base':
        this._pickBase(snap)
        return
      case 'pick-path':
        this._pickPath(ray)
        return
      case 'path-ready':
        this._commitPath()
        return
    }
  }

  // ----------------------------------------------------------------- keyboard

  onKey(ev: KeyboardEvent): void {
    if (!isBareModifier(ev.key)) this.note = ''
    if (ev.key === 'Escape') {
      this._escape()
      return
    }
    if (this.stage.kind === 'path-ready') {
      if (ev.key === 'Enter') this._commitPath()
      return
    }
    if (this.stage.kind === 'drag') {
      this._dragKey(ev)
      return
    }
    if (this.stage.kind === 'pick-face') this._retypeKey(ev)
  }

  private _escape(): void {
    const st = this.stage
    switch (st.kind) {
      case 'pick-object':
        this.cancel()
        return
      case 'pick-face':
        if (this.retype.isOpen) {
          this.disarmRetype()
          return
        }
        this.cancel()
        return
      case 'drag':
        this._resetGesture()
        this.stage = { kind: 'pick-face', object: st.object, box: st.box }
        this._showGizmo()
        return
      case 'pick-base':
        if (st.first !== null) {
          this.stage = { ...st, first: null }
          this.axisLine.clear()
          return
        }
        this.cancel()
        return
      case 'pick-path':
        this.axisLine.clear()
        this.pathLine.clear()
        this.stage = { kind: 'pick-base', object: st.object, first: null }
        return
      case 'path-ready':
        this._resetGesture()
        this.stage = { kind: 'pick-path', object: st.object, base: st.base }
        this._drawBase(st.base)
        return
    }
  }

  // ------------------------------------------------------------ stage: object

  private _acquireObject(ray: Ray): void {
    let nodes: NodeRef[] | null = null
    if (this.acquire !== null) nodes = this.acquire(ray)
    if (nodes === null) {
      // No host acquirer: fall back to the face under the cursor.
      try {
        const hit = pickFaceAt(this.wasmScene, ray)
        if (hit !== null && hit.instance === undefined) {
          nodes = [{ kind: 'object', id: hit.objectHandle }]
        }
      } catch {
        nodes = null
      }
    }
    if (nodes === null || nodes.length === 0) {
      this._notifyMiss('Click a solid to deform.')
      return
    }
    if (nodes.length !== 1 || nodes[0].kind !== 'object') {
      this._notifyMiss('Deform works on one solid at a time. Open the group or component and click the solid.')
      return
    }
    this.missNotified = false
    this.selection = nodes
    this._restart()
    if (this.stage.kind === 'pick-object') this._notifyMiss('That object has no shape to deform.')
  }

  private _notifyMiss(message: string): void {
    if (this.missNotified) return
    this.missNotified = true
    this.onToast(message)
  }

  // --------------------------------------------------- stage: face and the drag

  private _faceAt(ray: Ray): BoxFace | null {
    if (this.stage.kind !== 'pick-face') return null
    const { min, max } = this.stage.box
    const hit = rayBoxFace(ray, min, max)
    if (hit !== null) return hit
    // A grip can sit just outside the box silhouette; accept it too.
    const grip = this.grips === null ? null : this.gizmo.pick(ray, this.grips)
    if (grip === null) return null
    return boxFaces(min, max).find((f) => f.center.every((c, i) => Math.abs(c - grip.pos[i]) < 1e-9)) ?? null
  }

  private _hoverFace(ray: Ray): void {
    if (this.stage.kind !== 'pick-face') return
    const face = this._faceAt(ray)
    if (face === null) {
      this.hoverLine.clear()
      return
    }
    const { min, max } = this.stage.box
    this.hoverLine.set(faceOutline(min, max, face), pathHoverColors().hover, `${face.axisIndex}:${face.side}`)
  }

  private _beginDrag(face: BoxFace, ray: Ray): void {
    if (this.stage.kind !== 'pick-face') return
    const { object, box } = this.stage
    const frame = frameForFace(box.min, box.max, face)
    if (!(frame.length > 1e-6)) {
      this.onToast('That solid is too thin along that direction to deform.')
      return
    }
    const grab = hitMovingPlane(frame, ray) ?? movingCenter(frame)
    const n = normalizeV3([-ray.direction[0], -ray.direction[1], -ray.direction[2]]) ?? frame.axis
    const tracker = new TwistTracker()
    const c = movingCenter(frame)
    tracker.update(angleAround(frame, c, grab))
    const r0 = radiusFromAxis(frame, c, grab)
    this.problem = ''
    this.limitNote = ''
    this.stations = 0
    this.typed = ''
    this._hideGizmo()
    this.hoverLine.clear()
    this.stage = {
      kind: 'drag',
      object,
      box,
      drag: {
        face,
        frame,
        grab,
        dragNormal: n,
        tracker,
        grabRadius: r0 > 1e-6 ? r0 : null,
        bendU: null,
        amount: identityAmount(this.mode as BoxMode),
        limits: { neg: null, pos: null },
        limited: false,
      },
    }
    this._reportAmount()
  }

  private _dragTo(ray: Ray): void {
    if (this.stage.kind !== 'drag') return
    const d = this.stage.drag
    const mode = this.mode as BoxMode
    const c = movingCenter(d.frame)
    let amount = d.amount
    if (mode === 'twist') {
      const hit = hitMovingPlane(d.frame, ray)
      if (hit === null) return
      amount = d.tracker.update(angleAround(d.frame, c, hit))
    } else if (mode === 'taper') {
      const hit = hitMovingPlane(d.frame, ray)
      if (hit === null) return
      const r = radiusFromAxis(d.frame, c, hit)
      if (d.grabRadius === null) {
        if (r > 1e-6) d.grabRadius = r
        amount = 1
      } else {
        amount = taperRatio(r, d.grabRadius)
      }
    } else {
      const hit = rayPlaneIntersectAny(ray, d.grab, d.dragNormal)
      if (hit === null) return
      const delta = subV3(hit, d.grab)
      if (d.bendU === null) {
        if (Math.hypot(delta[0], delta[1], delta[2]) >= BEND_DECIDE_FRACTION * d.frame.length) {
          d.bendU = bendCrossAxis(delta, d.face.axisIndex)
          d.frame = { ...d.frame, u: d.bendU }
        }
      }
      amount = d.bendU === null ? 0 : bendAngle(delta, d.bendU, d.frame.length)
    }
    this._applyAmount(amount)
  }

  /** Set the amount, previewing it (clamped to what the kernel accepts). */
  private _applyAmount(amount: number): void {
    if (this.stage.kind !== 'drag') return
    const d = this.stage.drag
    if (Math.abs(amount - d.amount) < AMOUNT_EPS && this.ghost !== null) return
    d.amount = amount
    d.limited = false
    this.limitNote = ''
    this._previewAmount()
    if (this.typed === '') this._reportAmount()
    // A typed Bend's echo follows the side the pointer is now on, as Enter
    // will commit it.
    else this.onMeasurementCb(typedEcho(this.mode as BoxMode, this.typed, this.stage.drag.amount))
  }

  /** Draw the ghost for `drag.amount`; on SelfIntersecting, clamp to the
   *  kernel's limit (asked once per side per drag) and tint the ghost. An
   *  object too heavy to preview (`PreviewTooLarge`, from the preview or
   *  the limit search) gets no ghost and no clamp for the rest of the
   *  session, and the status says so. */
  private _previewAmount(): void {
    if (this.stage.kind !== 'drag') return
    const { object, drag: d } = this.stage
    const mode = this.mode as BoxMode
    this._forgetStaleSkips()
    if (this.tooLarge.has(object)) {
      this._clearGhost()
      return
    }
    const attempt = (amount: number): 'ok' | string => {
      const key = `${amount.toFixed(6)}:${this.stations}:${d.limited}:${d.frame.u.join(',')}`
      if (key === this.ghostKey && this.ghost !== null) return 'ok'
      try {
        const mesh = this.wasmScene.deform_preview(object, boxSpecJson(mode, d.frame, amount, this.stations))
        this._clearGhost()
        const ghost = buildMeshPreview(mesh, d.limited ? cueColors().blocked : GHOST_COLOR)
        if (ghost !== null) {
          this.ghost = ghost
          this.preview.add(ghost)
        }
        this.ghostKey = key
        return 'ok'
      } catch (err) {
        return parseKernelErrorCode(err) ?? 'Unknown'
      }
    }
    const skip = (): void => {
      this._forgetStaleSkips()
      this.tooLarge.add(object)
      this.limitNote = ''
      d.limited = false
      this._clearGhost()
    }
    const first = attempt(d.amount)
    if (first === 'ok') return
    if (first === 'PreviewTooLarge') {
      skip()
      return
    }
    if (first !== 'SelfIntersecting') {
      this._deformRefusedAtPreview(first)
      return
    }
    const id = identityAmount(mode)
    const side: 'neg' | 'pos' = d.amount >= id ? 'pos' : 'neg'
    if (d.limits[side] === null) {
      try {
        d.limits[side] = this.wasmScene.deform_preview_limit(object, boxSpecJson(mode, d.frame, d.amount, this.stations))
      } catch (err) {
        if (parseKernelErrorCode(err) === 'PreviewTooLarge') {
          skip()
          return
        }
        d.limits[side] = id
      }
    }
    const clamp = clampToLimit(mode, d.amount, d.limits[side] as number)
    d.limited = true
    d.amount = clamp.amount
    this.limitNote =
      mode === 'taper'
        ? `The result would pass through itself. Taper limited to ${formatPercent(clamp.amount)}.`
        : `The result would pass through itself. Angle limited to ${formatDegrees(Math.abs(clamp.amount))}.`
    const second = attempt(d.amount)
    if (second === 'PreviewTooLarge') skip()
    else if (second !== 'ok') this._clearGhost()
  }

  private _deformRefusedAtPreview(code: string): void {
    // A refusal that isn't "too far" (the object has gaps, a bad frame) won't
    // get better by moving: say why and back up to the face stage.
    this.problem = ''
    this.onToast(kernelErrorMessage(code, code), code)
    this._escape()
  }

  private _reportAmount(): void {
    if (this.stage.kind !== 'drag') return
    const a = this.stage.drag.amount
    // A bend's side is the drag's; the box shows how far, as a typed
    // amount means (`sidedAmount`).
    this.onMeasurementCb(
      this.mode === 'taper' ? formatPercent(a) : formatDegrees(this.mode === 'bend' ? Math.abs(a) : a),
    )
  }

  // ---------------------------------------------------------- typed amounts

  private _dragKey(ev: KeyboardEvent): void {
    if (this.stage.kind !== 'drag') return
    const mode = this.mode as BoxMode
    const drag = this.stage.drag
    if (ev.key === 'Enter') {
      const buf = this.typed
      if (buf === '') {
        this._commitDrag()
        return
      }
      const token = parseShapeToken(buf)
      if (token !== null && token.kind === 'count') {
        this.stations = token.n
        this.typed = ''
        this.onMeasurementCb('')
        this.note = `Stations set to ${token.n}.`
        this.noteRay = this.lastRay
        this.ghostKey = ''
        this._previewAmount()
        return
      }
      const amount = typedAmount(mode, buf)
      if (amount === null) {
        const why = amountMismatch(mode, buf)
        if (why !== '') this.onToast(why)
        return
      }
      this.typed = ''
      this._commitDrag(sidedAmount(mode, amount, drag.amount), buf)
      return
    }
    if (isShapeInputKey(ev.key)) {
      this.typed = editShapeBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(typedEcho(mode, this.typed, drag.amount))
    }
  }

  private _retypeKey(ev: KeyboardEvent): void {
    if (!this.retype.isOpen) return
    if (ev.key === 'Enter') {
      const buf = this.typed
      this.typed = ''
      this.onMeasurementCb('')
      const spec = this.retype.spec
      if (spec === null) return
      const token = parseShapeToken(buf)
      if (token !== null && token.kind === 'count') {
        this._committing(() => {
          if (this._retypeApply({ ...spec, stations: token.n })) {
            this.note = `Stations set to ${token.n}.`
            this.noteRay = this.lastRay
          }
        }, buf)
        return
      }
      const amount = typedAmount(spec.mode, buf)
      if (amount !== null) {
        this._committing(() => {
          this._retypeApply({ ...spec, amount: sidedAmount(spec.mode, amount, spec.amount) })
        }, buf)
        return
      }
      const why = amountMismatch(spec.mode, buf)
      if (why !== '') this.onToast(why)
      return
    }
    if (isShapeInputKey(ev.key)) {
      this.typed = editShapeBuffer(this.typed, ev.key, getLengthUnit())
      const spec = this.retype.spec
      this.onMeasurementCb(
        spec === null ? shapeReadout(this.typed, 'angle') : typedEcho(spec.mode, this.typed, spec.amount),
      )
    }
  }

  // ------------------------------------------------------------------ commits

  /** Commits the drag at `typedAmount` (a typed Enter) or the pointer's
   *  amount, as it is now: pointer moves while the busy card paints change
   *  the ghost, not what commits. `typed`: the buffer, restored if the
   *  commit cannot start. */
  private _commitDrag(typedAmount?: number, typed = ''): void {
    if (this.stage.kind !== 'drag') return
    const stage = this.stage
    const { object, box, drag: d } = stage
    const mode = this.mode as BoxMode
    const amount = typedAmount ?? d.amount
    if (Math.abs(amount - identityAmount(mode)) < AMOUNT_EPS) {
      // Nothing moved yet: a click that goes nowhere changes nothing.
      return
    }
    const asked: RetypeSpec = { object, mode, frame: { ...d.frame }, amount, stations: this.stations }
    this._committing(() => {
      // Nothing else runs while the card is up; should the gesture have
      // moved on anyway, commit nothing.
      if (this.stage !== stage) return
      const spec = this._invokeOrLimit(asked)
      if (spec === null) return
      this._changed(object)
      this._resetGesture()
      this.stage = { kind: 'pick-face', object, box }
      if (typeof this.wasmScene.history_generation === 'function') this.retype.arm(spec)
      this.onCommit(object)
      this._refreshBox()
    }, typed)
  }

  /** Runs a commit behind the busy card. A deform's time follows more than
   *  the object's size (a retry for thin facets, a limit search on a
   *  refusal), and a typed Enter right after the face pick has previewed
   *  nothing to judge by, so every commit gets the card and the app never
   *  freezes without a word. Without a host card it runs at once. */
  private _committing(work: () => void, typed = ''): void {
    if (this.busy === null) {
      work()
      return
    }
    if (this.busy(DEFORM_BUSY[this.mode], work) === false) {
      // Another long change holds the card: nothing ran. Keep what was
      // typed so Enter can try again, and say so.
      this.typed = typed
      if (typed === '' && this.stage.kind === 'drag') {
        this._reportAmount()
      } else if (typed !== '' && this.stage.kind === 'drag') {
        this.onMeasurementCb(typedEcho(this.mode as BoxMode, typed, this.stage.drag.amount))
      } else if (typed !== '' && this.retype.spec !== null) {
        this.onMeasurementCb(typedEcho(this.retype.spec.mode, typed, this.retype.spec.amount))
      }
      this.onToast(i18n._(DEFORM_BUSY_REFUSED))
    }
  }

  /** Forgets the skips when the document changed since they were learned
   *  (an undo or redo, an edit from the API or another tool). */
  private _forgetStaleSkips(): void {
    if (typeof this.wasmScene.history_generation !== 'function') return
    const g = this.wasmScene.history_generation()
    if (this.skipGeneration !== g) {
      this.tooLarge.clear()
      this.skipGeneration = g
    }
  }

  /** `object` changed shape: whether it previews in time is asked again. */
  private _changed(object: bigint): void {
    this.tooLarge.delete(object)
  }

  private _invoke(spec: RetypeSpec): void {
    const change = this.wasmScene.deform(
      spec.object,
      boxSpecJson(spec.mode, spec.frame, spec.amount, spec.stations),
    )
    change.free()
  }

  /** Redoes the last deform as `next`; true when the redo committed. */
  private _retypeApply(next: RetypeSpec): boolean {
    let committed = false
    // What was actually laid down: `next`, or `next` at the shape's limit.
    let used = next
    const run = (s: RetypeSpec, atLimitOk: boolean): boolean => {
      const done = atLimitOk ? this._invokeOrLimit(s) : this._invokeOrToast(s)
      if (done === null) return false
      committed = true
      used = done
      return true
    }
    const outcome = this.retype.apply(
      () => run(next, true),
      (s) => run(s, false),
      () => used,
    )
    if (outcome === 'stale') {
      this.onToast(retypeStaleMessage('deform'))
      return false
    }
    if (committed) {
      this._changed(next.object)
      this.onCommit(next.object)
      this._refreshBox()
    }
    return committed
  }

  /** The box changed with the shape: measure it again for the next gesture. */
  private _refreshBox(): void {
    if (this.stage.kind !== 'pick-face') return
    const box = this._boxOf(this.stage.object)
    if (box === null) return
    this.stage = { ...this.stage, box }
    this._showGizmo()
  }

  /** Commits `spec`. A typed amount within 1% of the largest the shape can
   *  take goes in at exactly that largest amount instead of being refused;
   *  any other refusal is toasted. Returns the spec laid down, or null. */
  private _invokeOrLimit(spec: RetypeSpec): RetypeSpec | null {
    try {
      this._invoke(spec)
      return spec
    } catch (err) {
      const near = this._specAtLimit(err, spec)
      if (near !== null) {
        try {
          this._invoke(near)
          return near
        } catch {
          // The exact limit is refused too: say why the typed amount was.
        }
      }
      this._toastRefusalFor(err, spec)
      return null
    }
  }

  /** Commits `spec`, toasting a refusal; the spec laid down, or null. */
  private _invokeOrToast(spec: RetypeSpec): RetypeSpec | null {
    try {
      this._invoke(spec)
      return spec
    } catch (err) {
      this._toastRefusalFor(err, spec)
      return null
    }
  }

  /** `spec` at the shape's limit when its amount is within 1% of that limit
   *  and was refused for passing through itself; otherwise null. */
  private _specAtLimit(err: unknown, spec: RetypeSpec): RetypeSpec | null {
    if (parseKernelErrorCode(err) !== 'SelfIntersecting') return null
    try {
      const limit = this._limitOf(err, spec)
      const id = identityAmount(spec.mode)
      if (!Number.isFinite(limit) || Math.abs(limit - id) < AMOUNT_EPS) return null
      if (Math.abs(spec.amount - limit) > 0.01 * Math.abs(limit - id)) return null
      return { ...spec, amount: limit }
    } catch {
      return null
    }
  }

  /** The limit a commit's `SelfIntersecting` refusal carries (the kernel
   *  searched for it while refusing), else the kernel's search for `spec`. */
  private _limitOf(err: unknown, spec: RetypeSpec): number {
    const carried = limitInRefusal(err)
    if (carried !== null) return carried
    return this.wasmScene.deform_limit(spec.object, boxSpecJson(spec.mode, spec.frame, spec.amount, spec.stations))
  }

  private _toastRefusal(err: unknown): void {
    const code = parseKernelErrorCode(err)
    const raw = err instanceof Error ? err.message : String(err)
    this.onToast(kernelErrorMessage(code ?? 'Unknown', raw), code ?? undefined)
  }

  /** A refusal of `spec`; an amount the shape cannot take without passing
   *  through itself names the largest it can, as the pointer's clamp does.
   *  A typed amount is never clamped for the user: it is refused, told. */
  private _toastRefusalFor(err: unknown, spec: RetypeSpec): void {
    if (parseKernelErrorCode(err) === 'SelfIntersecting') {
      try {
        const limit = this._limitOf(err, spec)
        const clamp = clampToLimit(spec.mode, spec.amount, limit)
        // A limit at the identity (nothing fits at all) names no size.
        if (Math.abs(clamp.amount - identityAmount(spec.mode)) < AMOUNT_EPS) throw new Error('no limit')
        this.onToast(
          spec.mode === 'taper'
            ? `The result would pass through itself. Taper limited to ${formatPercent(clamp.amount)}.`
            : `The result would pass through itself. Angle limited to ${formatDegrees(Math.abs(clamp.amount))}.`,
          'SelfIntersecting',
        )
        return
      } catch {
        // No limit to name: the plain refusal below.
      }
    }
    this._toastRefusal(err)
  }

  // ----------------------------------------------------- Bend to Path stages

  private _edgeAt(snap: Snap | null): { a: V3; b: V3; key: string } | null {
    if (snap === null || snap.kind !== 'on-edge' || snap.element === undefined) return null
    if (snap.instance !== undefined) return null
    try {
      let ends: ArrayLike<number> | undefined
      if (snap.elementKind === 'sketch-edge' && snap.sketch !== undefined) {
        ends = this.wasmScene.sketch_edge_endpoints(snap.sketch, snap.element)
      } else if (snap.elementKind === 'edge' && snap.object !== undefined) {
        ends = this.wasmScene.edge_endpoints(snap.object, snap.element)
      }
      if (ends === undefined || ends.length < 6) return null
      return {
        a: [ends[0], ends[1], ends[2]],
        b: [ends[3], ends[4], ends[5]],
        key: `${snap.elementKind}:${snap.sketch ?? snap.object}:${snap.element}`,
      }
    } catch {
      return null
    }
  }

  private _hoverBase(snap: Snap | null): void {
    if (this.stage.kind !== 'pick-base') return
    const { first } = this.stage
    if (snap === null) {
      this.axisLine.clear()
      return
    }
    const edge = first === null ? this._edgeAt(snap) : null
    if (edge !== null) {
      this.axisLine.set([...edge.a, ...edge.b], pathHoverColors().hover, edge.key)
      return
    }
    if (first === null) {
      this.axisLine.clear()
      return
    }
    this.axisLine.set([...first, snap.x, snap.y, snap.z], pathHoverColors().hover)
  }

  private _pickBase(snap: Snap | null): void {
    if (this.stage.kind !== 'pick-base' || snap === null) return
    const { object, first } = this.stage
    const edge = first === null ? this._edgeAt(snap) : null
    if (edge !== null) {
      if (axisFromPoints(edge.a, edge.b) === null) return
      this._enterPath(object, { from: edge.a, to: edge.b })
      return
    }
    const p: V3 = [snap.x, snap.y, snap.z]
    if (first === null) {
      this.stage = { kind: 'pick-base', object, first: p }
      return
    }
    if (axisFromPoints(first, p) === null) return
    this._enterPath(object, { from: first, to: p })
  }

  private _enterPath(object: bigint, base: { from: V3; to: V3 }): void {
    this.stage = { kind: 'pick-path', object, base }
    this._drawBase(base)
  }

  private _drawBase(base: { from: V3; to: V3 }): void {
    this.axisLine.set([...base.from, ...base.to], pathHoverColors().path)
  }

  /** The path under the cursor as a polyline, oriented so the base line's
   *  start maps to the nearer end of an open path. */
  private _pathAt(ray: Ray, base: { from: V3; to: V3 }): PolylinePath | null {
    try {
      const edges = pickEdgePath(this.wasmScene, ray)
      let path: PolylinePath | null = null
      if (edges !== null) {
        path = edgePathPolyline(this.wasmScene, edges.sketchHandle, edges.edgeHandles)
      } else {
        const hit = pickFaceAt(this.wasmScene, ray)
        if (hit !== null && hit.instance === undefined) {
          path = loopPolyline(this.wasmScene.face_boundary(hit.objectHandle, hit.faceHandle))
        }
      }
      if (path === null) return null
      if (!path.closed && path.points.length > 1) {
        const first = path.points[0]
        const last = path.points[path.points.length - 1]
        const dist = (p: V3) => Math.hypot(p[0] - base.from[0], p[1] - base.from[1], p[2] - base.from[2])
        if (dist(last) < dist(first)) path = { ...path, points: [...path.points].reverse() }
      }
      return path
    } catch {
      return null
    }
  }

  /** Whether the drawn chain under `ray` is there but cannot be read as
   *  one run (it branches). */
  private _pathForks(ray: Ray): boolean {
    try {
      const edges = pickEdgePath(this.wasmScene, ray)
      return edges !== null && edgePathPolyline(this.wasmScene, edges.sketchHandle, edges.edgeHandles) === null
    } catch {
      return false
    }
  }

  private _hoverPath(ray: Ray): void {
    if (this.stage.kind !== 'pick-path') return
    const path = this._pathAt(ray, this.stage.base)
    if (path === null) {
      this.hoverLine.clear()
      return
    }
    this.hoverLine.set(polylineSegments(path), pathHoverColors().hover, JSON.stringify(path.points[0]) + path.points.length)
  }

  private _pickPath(ray: Ray): void {
    if (this.stage.kind !== 'pick-path') return
    const { object, base } = this.stage
    const path = this._pathAt(ray, base)
    if (path === null) {
      // A drawn chain under the pointer that will not chain into one run
      // forks (three or more lines meet somewhere in it): say so on every
      // click, as Follow Me does, instead of the aim-here hint.
      if (this._pathForks(ray)) {
        this.onToast(PATH_FORKS)
        return
      }
      this._notifyMiss('Click a drawn line or curve for the path.')
      return
    }
    this.missNotified = false
    this.hoverLine.clear()
    this.pathLine.set(polylineSegments(path), pathHoverColors().path)
    this.stage = { kind: 'path-ready', object, base, path }
    this._previewPath()
  }

  private _pathSpec(): string | null {
    if (this.stage.kind !== 'path-ready') return null
    const { base, path } = this.stage
    return pathSpecJson(base, path.points, path.closed, this.stations)
  }

  private _previewPath(): void {
    const json = this._pathSpec()
    if (json === null || this.stage.kind !== 'path-ready') return
    this.problem = ''
    try {
      const mesh = this.wasmScene.deform_preview(this.stage.object, json)
      this._clearGhost()
      const ghost = buildMeshPreview(mesh, GHOST_COLOR)
      if (ghost !== null) {
        this.ghost = ghost
        this.preview.add(ghost)
      }
    } catch (err) {
      this._clearGhost()
      const code = parseKernelErrorCode(err)
      if (code === 'PreviewTooLarge') this.tooLarge.add(this.stage.object)
      this.problem = kernelErrorMessage(code ?? 'Unknown', err instanceof Error ? err.message : String(err))
    }
  }

  private _commitPath(): void {
    const json = this._pathSpec()
    if (json === null || this.stage.kind !== 'path-ready') return
    const stage = this.stage
    const { object } = stage
    this._committing(() => {
      if (this.stage !== stage) return
      try {
        this.wasmScene.deform(object, json).free()
      } catch (err) {
        this._toastRefusal(err)
        return
      }
      this._changed(object)
      this._resetGesture()
      this.hoverLine.clear()
      this.stage = { kind: 'pick-base', object, first: null }
      this.onCommit(object)
    })
  }

  // ---------------------------------------------------------------- clearing

  private _clearGhost(): void {
    if (this.ghost === null) return
    this.preview.remove(this.ghost)
    this.ghost.traverse((c) => {
      if (c instanceof THREE.Mesh || c instanceof THREE.LineSegments) {
        c.geometry.dispose()
        const m = c.material
        if (Array.isArray(m)) m.forEach((x) => x.dispose())
        else m.dispose()
      }
    })
    this.ghost = null
    this.ghostKey = ''
  }
}

/** The ray's hit on a plane, also when the plane lies behind the eye. */
function rayPlaneIntersectAny(ray: Ray, point: V3, normal: V3): V3 | null {
  const fwd = rayPlaneIntersect(ray.origin as V3, ray.direction as V3, point, normal)
  if (fwd !== null) return fwd
  const denom =
    ray.direction[0] * normal[0] + ray.direction[1] * normal[1] + ray.direction[2] * normal[2]
  if (Math.abs(denom) < 1e-10) return null
  const t =
    ((point[0] - ray.origin[0]) * normal[0] +
      (point[1] - ray.origin[1]) * normal[1] +
      (point[2] - ray.origin[2]) * normal[2]) /
    denom
  return [
    ray.origin[0] + t * ray.direction[0],
    ray.origin[1] + t * ray.direction[1],
    ray.origin[2] + t * ray.direction[2],
  ]
}

/** The 4 segments around a box face. */
function faceOutline(min: V3, max: V3, face: BoxFace): number[] {
  const i = face.axisIndex
  const a = ((i + 1) % 3) as 0 | 1 | 2
  const b = ((i + 2) % 3) as 0 | 1 | 2
  const at = (ua: number, ub: number): V3 => {
    const p: V3 = [0, 0, 0]
    p[i] = face.side === 0 ? min[i] : max[i]
    p[a] = ua
    p[b] = ub
    return p
  }
  const c = [at(min[a], min[b]), at(max[a], min[b]), at(max[a], max[b]), at(min[a], max[b])]
  const out: number[] = []
  for (let k = 0; k < 4; k++) out.push(...c[k], ...c[(k + 1) % 4])
  return out
}

/** A typed amount in the mode's own unit: degrees (bare, `°` or `deg`) for
 *  Twist and Bend, percent (bare or `%`) for Taper. Null when it doesn't
 *  parse or is outside what the kernel accepts. */
/** Why a typed amount doesn't fit the mode (a percentage for a twist, an
 *  angle for a taper), for a toast; '' when the buffer is not that case. */
export function amountMismatch(mode: BoxMode, buf: string): string {
  const t = parseShapeToken(buf)
  if (mode !== 'taper') {
    const a = parseAngleBuffer(buf)
    if (a !== null && Math.abs(a.radians) > (mode === 'twist' ? FOUR_PI : TWO_PI)) {
      return mode === 'twist' ? 'Twist takes up to 720°.' : 'Bend takes up to 360°.'
    }
  }
  if (mode === 'taper' && t?.kind === 'angle') return 'Taper takes a percentage, like 60%.'
  if (mode !== 'taper' && t?.kind === 'percent') return 'Only a taper takes a percentage. Type an angle like 25°.'
  return ''
}

export function typedAmount(mode: BoxMode, buf: string): number | null {
  if (mode === 'taper') {
    const r = parseRatioBuffer(buf)
    return r !== null && r > 0 ? r : null
  }
  const a = parseAngleBuffer(buf)
  if (a === null) return null
  const limit = mode === 'twist' ? FOUR_PI : TWO_PI
  return Math.abs(a.radians) <= limit ? a.radians : null
}

/** A Bend's typed amount on the side the drag (or the deform being redone)
 *  is on: the drag picks the side, the number says how far, and an explicit
 *  minus flips it, as Push/Pull's typed distance does. Twist and Taper
 *  amounts stand as typed (a twist's sign is its turn). */
export function sidedAmount(mode: BoxMode, typed: number, current: number): number {
  return mode === 'bend' && current < 0 ? -typed : typed
}

/** The limit a `SelfIntersecting` refusal names (`… (limit 0.52)`), or null. */
export function limitInRefusal(err: unknown): number | null {
  const msg = err instanceof Error ? err.message : String(err)
  const m = /\(limit (-?[0-9][0-9.eE+-]*)\)/.exec(msg)
  if (m === null) return null
  const v = Number(m[1])
  return Number.isFinite(v) ? v : null
}

/** What the measurement box echoes for a typed buffer: the buffer as typed,
 *  except a Bend's, which shows how far and says when it bends the other
 *  way from the drag (or the bend being redone), as `sidedAmount` will
 *  commit it. */
export function typedEcho(mode: BoxMode, buf: string, current: number): string {
  const plain = shapeReadout(buf, mode === 'taper' ? 'percent' : 'angle')
  if (mode !== 'bend' || current === 0) return plain
  const a = parseAngleBuffer(buf)
  if (a === null) return plain
  const unsigned = plain.replace(/^-/, '')
  const flipped = Math.sign(sidedAmount(mode, a.radians, current)) !== Math.sign(current) && a.radians !== 0
  return flipped ? `${unsigned} the other way` : unsigned
}
