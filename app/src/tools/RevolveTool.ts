/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * RevolveTool — spin a profile around an axis into a solid (true-curves.md
 * §5.1), the lathe: a rectangle becomes a cylinder, a half-circle a sphere,
 * a goblet outline a goblet.
 *
 * Gesture:
 *   1. Click the profile: a drawn shape (sketch region) or a solid face. The
 *      shape under the cursor is outlined first, as in Follow Me. A drawn
 *      shape selected before the tool starts is the profile already: the
 *      tool opens at stage 2. (A solid face cannot be selected, so a face
 *      is always picked here.)
 *   2. Pick the axis: click an edge (a drawn line or a solid's edge) to use
 *      that line, or click two points. Only an axis that lies in the
 *      profile's plane is accepted; anything else shows in the "blocked"
 *      color with a plain reason.
 *   3. Set the angle. The ghost shows the full turn at once; moving the
 *      pointer turns it (15° snaps), a click finishes at that angle, Enter
 *      finishes the full turn. Typing `90`, `90°` or `90deg` finishes at
 *      that angle; `24s` sets the segments around a full turn.
 *   Esc steps back one stage. After the commit, an angle or segment count
 *   typed redoes the revolve (the shared retype window).
 *
 * The kernel owns every decision (the axis in the plane, the profile not
 * crossing the axis, self-intersection); the ghost is the kernel's own
 * `revolve_preview`, and a refusal is shown with the shared kernel-error copy.
 */

import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import { sameRay, type Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { defaultFaceEligible, type FaceEligible } from './faceDraw'
import { clearPreview } from './transformPreview'
import { getLengthUnit } from '../settings/units'
import { applyAffine3x4 } from '../viewport/geoHelpers'
import { pickRegionAt, pickFaceAt, regionRing, ringSegments, rayAtProfile } from './profilePick'
import { isDraftRef, readPlaneUnits, regionRingEdges, resolveDraft } from './planeUnits'
import { LineSlot } from './previewLines'
import { buildMeshPreview } from './meshPreview'
import { cueColors, pathHoverColors } from './FollowMeTool'
import { RetypeWindow, idleRetypeCapturesKey, retypeStaleMessage } from './retypeWindow'
import {
  editShapeBuffer,
  isBareModifier,
  isShapeInputKey,
  parseAngleBuffer,
  parseShapeToken,
  shapeReadout,
  formatDegrees,
} from './shapeInput'
import {
  FULL_TURN,
  axisFromPoints,
  axisInPlane,
  pointerAngle,
  referencePoint,
  revolveFrame,
  revolveProfileJson,
  revolveSegments,
  revolveSpecJson,
  SEGMENTS_RANGE,
  segmentsInRange,
  snapAngle,
  type PlaneDef,
  type RevolveFrame,
  type V3,
} from './revolveMath'

export type OnRevolveCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

const AXIS_NOT_IN_PLANE = 'The axis has to lie in the plane of the shape. Move the axis onto that plane.'
const GHOST_COLOR = 0xee8800
/** Angles closer than this to the previewed one reuse the preview (radians). */
const ANGLE_EPS = 1e-4

type Profile =
  | { kind: 'region'; sketch: bigint; region: bigint }
  | { kind: 'face'; object: bigint; face: bigint; instance: bigint | undefined }

interface ProfileInfo {
  profile: Profile
  ring: V3[]
  plane: PlaneDef
}

type Stage =
  | { kind: 'pick-profile' }
  | { kind: 'axis'; info: ProfileInfo; first: V3 | null }
  | {
      kind: 'angle'
      info: ProfileInfo
      axisPoint: V3
      axisDir: V3
      frame: RevolveFrame
      /** Radians; a full turn until the pointer says otherwise. */
      angle: number
      /** Segments around a full turn. */
      segments: number
    }

/** What a retype redoes: the exact revolve, with a new angle or segments. */
interface RetypeSpec {
  info: ProfileInfo
  axisPoint: V3
  axisDir: V3
  angle: number
  segments: number
}

export class RevolveTool implements Tool {
  readonly name = 'Revolve'

  private stage: Stage = { kind: 'pick-profile' }
  private readonly retype: RetypeWindow<RetypeSpec>
  private typed = ''
  /** A one-line confirmation ("Segments set to 24.") shown until the next
   *  pointer move or key. `noteRay` is the ray it was set under: the
   *  Viewport re-sends that ray after a key, which is not a move. */
  private note = ''
  private noteRay: Ray | null = null
  private lastRay: Ray | null = null

  private readonly profileLine: LineSlot
  private readonly hoverLine: LineSlot
  private readonly axisLine: LineSlot
  private ghost: THREE.Group | null = null
  private ghostKey = ''

  /** The profile under the cursor at the first stage. */
  private hovered: ProfileInfo | null = null
  /** Why the axis under the cursor can't be used ('' when it can). */
  private axisProblem = ''
  /** The kernel's reason the ghost couldn't be built ('' when it was). */
  private previewProblem = ''
  private missNotified = false

  private _editContext: EditContext = { kind: 'top' }
  /** True once `setEditContext` has been called. The Viewport always calls it
   *  once right after constructing a tool, to prime it with the CURRENT
   *  context; that first call only assigns (as FollowMeTool's does), so a
   *  preselection made inside a group, object, or component edit context is
   *  not cancelled the instant the tool is built. */
  private _editContextPrimed = false
  /** The selection the tool was opened with, and the profile it resolved to
   *  (null when none): a context change before any gesture begins re-resolves
   *  the preselection instead of dropping it. */
  private readonly _initialSelection: readonly NodeRef[]
  private _preselected: ProfileInfo | null = null
  private _faceEligible: FaceEligible | null = null

  constructor(
    private readonly wasmScene: WasmScene,
    private readonly preview: THREE.Group,
    private readonly onCommit: OnRevolveCommit,
    private readonly onToast: OnToast,
    private readonly onMeasurementCb: OnMeasurement = () => { /* no-op */ },
    initialSelection: readonly NodeRef[] = [],
  ) {
    this.retype = new RetypeWindow(wasmScene)
    this.profileLine = new LineSlot(preview, 996)
    this.hoverLine = new LineSlot(preview, 995)
    this.axisLine = new LineSlot(preview, 997)

    this._initialSelection = initialSelection
    const preselected = this._profileFromSelection(initialSelection)
    if (preselected !== null) {
      this._preselected = preselected
      this.stage = { kind: 'axis', info: preselected, first: null }
    }
  }

  /**
   * Re-draw the preselected profile's outline once the tool is active.
   * `ToolController` builds the new tool BEFORE the outgoing tool's
   * `cancel()` clears the shared preview group, so the constructor's outline
   * would be wiped on a tool-to-tool switch (see FollowMeTool.activate).
   * Idempotent.
   */
  activate(): void {
    if (this.stage.kind === 'axis') this.profileLine.set(ringSegments(this.stage.info.ring), pathHoverColors().path)
  }

  /**
   * Resolve a preselection into a profile: exactly one selected drawn
   * shape that is a single closed region of a free sketch (a filled region,
   * or the island or circle whose outline is exactly one such region).
   * Two selections, a solid, or anything unresolved yields nothing, and the
   * tool starts at the profile pick as before.
   */
  private _profileFromSelection(selection: readonly NodeRef[]): ProfileInfo | null {
    if (selection.length !== 1 || !isDraftRef(selection[0])) return null
    const ref = selection[0]
    try {
      const pu = readPlaneUnits(this.wasmScene, ref.ctx)
      if (pu === null) return null
      let region
      if (ref.kind === 'draft-region') {
        region = pu.regionByKey.get(ref.key)
      } else {
        const drawn = resolveDraft(this.wasmScene, ref)
        if (drawn === null) return null
        const keys = new Set(drawn.edges.map((e) => e.key))
        const closed = pu.regions.filter((r) => {
          if (r.extrude === null) return false
          const ring = regionRingEdges(pu, r, true)
          return ring.length > 0 && ring.every((e) => keys.has(e.key))
        })
        if (closed.length === 1) region = closed[0]
      }
      if (region === undefined || region.extrude === null) return null
      const { sketch, region: handle } = region.extrude
      const pl = this.wasmScene.sketch_plane(sketch)
      if (pl === undefined || pl.length < 6) return null
      const ring = regionRing(this.wasmScene, sketch, handle)
      if (ring.length < 3) return null
      return {
        profile: { kind: 'region', sketch, region: handle },
        ring,
        plane: { point: [pl[0], pl[1], pl[2]], normal: [pl[3], pl[4], pl[5]] },
      }
    } catch {
      return null // stale handle — no usable preselection
    }
  }

  // ------------------------------------------------------------- Tool plumbing

  setEditContext(ctx: EditContext): void {
    if (!this._editContextPrimed) {
      this._editContextPrimed = true
      this._editContext = ctx
      return
    }
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
    // Still at the untouched preselected axis stage: no gesture is begun, so
    // re-resolve the preselection under the new context instead of dropping it.
    if (this.stage.kind === 'axis' && this.stage.first === null && this.stage.info === this._preselected && this.typed === '') {
      const again = this._profileFromSelection(this._initialSelection)
      if (again !== null) {
        this._preselected = again
        this.stage = { kind: 'axis', info: again, first: null }
        this.activate()
        return
      }
    }
    this.cancel()
  }

  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }

  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
  }

  private get _activeObjectContext(): bigint | null {
    return this._editContext.kind === 'object' ? this._editContext.id : null
  }

  capturingInput(): boolean {
    return this.stage.kind !== 'pick-profile'
  }

  hasArmedGesture(): boolean {
    return this.capturingInput() || this.typed !== ''
  }

  capturesKey(key: string): boolean {
    if (this.stage.kind === 'angle') return key === 'Enter' || isShapeInputKey(key)
    if (this.stage.kind === 'axis') return false
    return this.retype.isOpen && idleRetypeCapturesKey(key, this.typed, isShapeInputKey)
  }

  disarmRetype(): void {
    this.retype.close()
    if (this.stage.kind === 'pick-profile' && this.typed !== '') {
      this.typed = ''
      this.onMeasurementCb('')
    }
  }

  onHistoryChanged(): void {
    this.retype.close()
  }

  onDocumentReset(): void {
    this.cancel()
  }

  statusHint(): string {
    // A problem outranks the confirmation note; the note outranks the hint.
    switch (this.stage.kind) {
      case 'pick-profile':
        if (this.note !== '') return this.note
        return this.retype.isOpen
          ? 'Type an angle (90°) or segments (24s) to redo the revolve, or click the next shape.'
          : 'Click a drawn shape or a solid face to revolve.'
      case 'axis':
        if (this.axisProblem !== '') return this.axisProblem
        if (this.note !== '') return this.note
        return this.stage.first === null
          ? 'Click an edge or two points for the axis.'
          : 'Click the second point of the axis.'
      case 'angle':
        if (this.previewProblem !== '') return this.previewProblem
        if (this.note !== '') return this.note
        return 'Move to set the angle, click to finish, Enter for a full turn.'
    }
  }

  cancel(): void {
    this.stage = { kind: 'pick-profile' }
    this.typed = ''
    this.note = ''
    this.hovered = null
    this.axisProblem = ''
    this.previewProblem = ''
    this.missNotified = false
    this.retype.close()
    this._clearAll()
    this.onMeasurementCb('')
    clearPreview(this.preview)
  }

  // ------------------------------------------------------------------ pointer

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (!sameRay(this.noteRay, ray)) this.note = ''
    this.lastRay = ray
    switch (this.stage.kind) {
      case 'pick-profile':
        this._hoverProfile(ray)
        return
      case 'axis':
        this._hoverAxis(snap)
        return
      case 'angle': {
        const raw = pointerAngle(this.stage.frame, ray)
        if (raw === null) return
        this._setAngle(snapAngle(raw))
        return
      }
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    this.note = ''
    this.disarmRetype()
    switch (this.stage.kind) {
      case 'pick-profile':
        this._pickProfile(ray)
        return
      case 'axis':
        this._pickAxis(snap)
        return
      case 'angle': {
        const raw = pointerAngle(this.stage.frame, ray)
        if (raw !== null) this._setAngle(snapAngle(raw))
        this._commitCurrent(this.stage.angle)
        return
      }
    }
  }

  /** A double-click finishes the full turn. */
  onDoubleClick(): boolean {
    if (this.stage.kind !== 'angle') return false
    this._commitCurrent(FULL_TURN)
    return true
  }

  // ---------------------------------------------------------------- keyboard

  onKey(ev: KeyboardEvent): void {
    if (!isBareModifier(ev.key)) this.note = ''
    if (ev.key === 'Escape') {
      if (this.stage.kind === 'pick-profile') {
        if (this.retype.isOpen) this.disarmRetype()
        else this.cancel()
        return
      }
      if (this.stage.kind === 'angle') {
        const { info } = this.stage
        this.typed = ''
        this.onMeasurementCb('')
        this._clearGhost()
        this.axisLine.clear()
        this.previewProblem = ''
        this.stage = { kind: 'axis', info, first: null }
        return
      }
      // axis → back to the profile pick.
      this.stage = { kind: 'pick-profile' }
      this.axisProblem = ''
      this.profileLine.clear()
      this.axisLine.clear()
      this.hoverLine.clear()
      return
    }

    if (this.stage.kind === 'angle') {
      this._angleKey(ev)
      return
    }
    if (this.stage.kind === 'pick-profile') this._retypeKey(ev)
  }

  private _angleKey(ev: KeyboardEvent): void {
    if (this.stage.kind !== 'angle') return
    if (ev.key === 'Enter') {
      const buf = this.typed
      if (buf === '') {
        this._commitCurrent(FULL_TURN)
        return
      }
      const token = parseShapeToken(buf)
      if (token !== null && token.kind === 'count') {
        // Segments: reshape the ghost and keep going.
        this.typed = ''
        this.onMeasurementCb('')
        if (!segmentsInRange(token.n)) {
          this.onToast(SEGMENTS_RANGE)
          return
        }
        this.stage = { ...this.stage, segments: token.n }
        this.note = `Segments set to ${token.n}.`
        this.noteRay = this.lastRay
        this._updatePreview()
        return
      }
      const angle = parseAngleBuffer(buf)
      if (angle === null) return
      // The buffer is spent either way: a refused angle must not prefix the
      // next one typed.
      this.typed = ''
      this.onMeasurementCb('')
      this._commitCurrent(angle.radians)
      return
    }
    if (isShapeInputKey(ev.key)) {
      this.typed = editShapeBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(shapeReadout(this.typed, 'angle'))
    }
  }

  private _retypeKey(ev: KeyboardEvent): void {
    if (!this.retype.isOpen) return
    if (ev.key === 'Enter') {
      const buf = this.typed
      this.typed = ''
      this.onMeasurementCb('')
      const token = parseShapeToken(buf)
      if (token !== null && token.kind === 'count') {
        if (!segmentsInRange(token.n)) {
          this.onToast(SEGMENTS_RANGE)
          return
        }
        if (this._retypeApply({ segments: token.n })) {
          this.note = `Segments set to ${token.n}.`
          this.noteRay = this.lastRay
        }
        return
      }
      const angle = parseAngleBuffer(buf)
      if (angle !== null) this._retypeApply({ angle: angle.radians })
      return
    }
    if (isShapeInputKey(ev.key)) {
      this.typed = editShapeBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(shapeReadout(this.typed, 'angle'))
    }
  }

  // -------------------------------------------------------------- stage 1: profile

  private _faceOk(object: bigint, instance: bigint | undefined): boolean {
    const active = this._activeInstance
    if (active !== null) return instance === active
    if (instance !== undefined) return false
    return this._faceEligible !== null
      ? this._faceEligible(object, instance)
      : defaultFaceEligible(this.wasmScene, this._activeObjectContext, object, instance)
  }

  /** The profile under `ray` (a drawn region first, then an eligible face),
   *  with its outline and plane; null for nothing usable. */
  private _profileAt(ray: Ray): { info: ProfileInfo | null; problem: string } {
    const active = this._activeInstance
    try {
      const region = pickRegionAt(this.wasmScene, ray, active)
      if (region !== null) {
        if (active !== null) {
          return {
            info: null,
            problem: 'Step out of the component to revolve a drawn shape, or revolve one of its faces.',
          }
        }
        const pl = this.wasmScene.sketch_plane(region.sketchHandle)
        if (pl === undefined || pl.length < 6) return { info: null, problem: '' }
        const ring = regionRing(this.wasmScene, region.sketchHandle, region.regionHandle)
        if (ring.length < 3) return { info: null, problem: '' }
        return {
          info: {
            profile: { kind: 'region', sketch: region.sketchHandle, region: region.regionHandle },
            ring,
            plane: { point: [pl[0], pl[1], pl[2]], normal: [pl[3], pl[4], pl[5]] },
          },
          problem: '',
        }
      }
      const face = pickFaceAt(this.wasmScene, ray)
      if (face === null) return { info: null, problem: '' }
      if (!this._faceOk(face.objectHandle, face.instance)) {
        return { info: null, problem: 'Enter the group or component to revolve that face, or pick one outside it.' }
      }
      return { info: this._faceInfo(face.objectHandle, face.faceHandle, face.instance), problem: '' }
    } catch {
      return { info: null, problem: '' }
    }
  }

  private _faceInfo(object: bigint, face: bigint, instance: bigint | undefined): ProfileInfo | null {
    const loop = this.wasmScene.face_boundary(object, face)
    const pl = this.wasmScene.face_plane(object, face)
    if (pl === undefined || pl.length < 6 || loop.length < 9) return null
    let ring: V3[] = []
    for (let i = 0; i + 2 < loop.length; i += 3) ring.push([loop[i], loop[i + 1], loop[i + 2]])
    let plane: PlaneDef = { point: [pl[0], pl[1], pl[2]], normal: [pl[3], pl[4], pl[5]] }
    const active = this._activeInstance
    if (active !== null) {
      // A definition member's geometry is definition-local; the axis and the
      // ghost live in world space, so pose the face into it.
      const pose = this.wasmScene.instance_pose(active)
      if (pose === undefined) return null
      ring = ring.map((p) => applyAffine3x4(pose, p))
      const n = plane.normal
      const origin = applyAffine3x4(pose, [0, 0, 0])
      const tip = applyAffine3x4(pose, [n[0], n[1], n[2]])
      const wn: V3 = [tip[0] - origin[0], tip[1] - origin[1], tip[2] - origin[2]]
      const len = Math.hypot(wn[0], wn[1], wn[2])
      plane = { point: applyAffine3x4(pose, plane.point), normal: len > 0 ? [wn[0] / len, wn[1] / len, wn[2] / len] : n }
    }
    return { profile: { kind: 'face', object, face, instance }, ring, plane }
  }

  private _hoverProfile(ray: Ray): void {
    const { info } = this._profileAt(ray)
    this.hovered = info
    if (info === null) {
      this.hoverLine.clear()
      return
    }
    const key =
      info.profile.kind === 'region'
        ? `r:${info.profile.sketch}:${info.profile.region}`
        : `f:${info.profile.object}:${info.profile.face}`
    this.hoverLine.set(ringSegments(info.ring), pathHoverColors().hover, key)
  }

  private _pickProfile(ray: Ray): void {
    const { info, problem } = this._profileAt(ray)
    if (info === null) {
      if (problem !== '') this._notifyMiss(problem)
      else this._notifyMiss('Click a drawn shape or a solid face to revolve.')
      return
    }
    this.missNotified = false
    this.hoverLine.clear()
    this.profileLine.set(ringSegments(info.ring), pathHoverColors().path)
    this.hovered = null
    this.stage = { kind: 'axis', info, first: null }
    this.axisProblem = ''
  }

  private _notifyMiss(message: string): void {
    if (this.missNotified) return
    this.missNotified = true
    this.onToast(message)
  }

  // ------------------------------------------------------------------ stage 2: axis

  /** The straight line of the edge a snap sits on, when it is on a drawn or
   *  solid edge in the world. */
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

  private _hoverAxis(snap: Snap | null): void {
    if (this.stage.kind !== 'axis') return
    const { info, first } = this.stage
    this.axisProblem = ''
    if (snap === null) {
      this.axisLine.clear()
      return
    }
    const blocked = cueColors().blocked
    const ok = pathHoverColors().hover
    const edge = first === null ? this._edgeAt(snap) : null
    if (edge !== null) {
      const fine = axisFromPoints(edge.a, edge.b) !== null && axisInPlane(edge.a, edge.b, info.plane)
      if (!fine) this.axisProblem = AXIS_NOT_IN_PLANE
      this.axisLine.set([...edge.a, ...edge.b], fine ? ok : blocked, `${edge.key}:${fine}`)
      return
    }
    const p: V3 = [snap.x, snap.y, snap.z]
    if (first === null) {
      // First point: a marker-less cue; the line appears once there are two.
      const inPlane = axisInPlane(p, p, info.plane)
      if (!inPlane) this.axisProblem = AXIS_NOT_IN_PLANE
      this.axisLine.clear()
      return
    }
    const line = axisFromPoints(first, p)
    const fine = line !== null && axisInPlane(first, p, info.plane)
    if (line !== null && !fine) this.axisProblem = AXIS_NOT_IN_PLANE
    this.axisLine.set([...first, ...p], fine ? ok : blocked)
  }

  private _pickAxis(snap: Snap | null): void {
    if (this.stage.kind !== 'axis' || snap === null) return
    const { info, first } = this.stage
    const edge = first === null ? this._edgeAt(snap) : null
    if (edge !== null) {
      const line = axisFromPoints(edge.a, edge.b)
      if (line === null || !axisInPlane(edge.a, edge.b, info.plane)) {
        this.onToast(AXIS_NOT_IN_PLANE)
        return
      }
      this._enterAngle(info, line.point, line.dir)
      return
    }
    const p: V3 = [snap.x, snap.y, snap.z]
    if (first === null) {
      if (!axisInPlane(p, p, info.plane)) {
        this.onToast(AXIS_NOT_IN_PLANE)
        return
      }
      this.stage = { kind: 'axis', info, first: p }
      return
    }
    const line = axisFromPoints(first, p)
    if (line === null) return // the same point twice: wait for a second point
    if (!axisInPlane(first, p, info.plane)) {
      this.onToast(AXIS_NOT_IN_PLANE)
      return
    }
    this._enterAngle(info, line.point, line.dir)
  }

  // ------------------------------------------------------------------ stage 3: angle

  private _enterAngle(info: ProfileInfo, axisPoint: V3, axisDir: V3): void {
    const ref = referencePoint(info.ring, axisPoint, axisDir)
    const frame = ref === null ? null : revolveFrame(axisPoint, axisDir, ref)
    if (frame === null) {
      this.onToast('The axis passes through the shape. Pick an axis beside it or along its edge.')
      return
    }
    this.axisProblem = ''
    this.previewProblem = ''
    this.stage = {
      kind: 'angle',
      info,
      axisPoint,
      axisDir: frame.axis,
      frame,
      angle: FULL_TURN,
      segments: revolveSegments(info.ring, axisPoint, frame.axis),
    }
    this._drawAxis()
    this._updatePreview()
    this._reportAngle()
  }

  private _drawAxis(): void {
    if (this.stage.kind !== 'angle') return
    const { info, axisPoint, axisDir } = this.stage
    // Span the profile along the axis, with some margin either side.
    let lo = Infinity
    let hi = -Infinity
    for (const p of info.ring) {
      const t = (p[0] - axisPoint[0]) * axisDir[0] + (p[1] - axisPoint[1]) * axisDir[1] + (p[2] - axisPoint[2]) * axisDir[2]
      lo = Math.min(lo, t)
      hi = Math.max(hi, t)
    }
    const margin = Math.max(0.1 * (hi - lo), 0.05)
    lo -= margin
    hi += margin
    const at = (t: number): number[] => [
      axisPoint[0] + axisDir[0] * t,
      axisPoint[1] + axisDir[1] * t,
      axisPoint[2] + axisDir[2] * t,
    ]
    this.axisLine.set([...at(lo), ...at(hi)], pathHoverColors().path)
  }

  private _setAngle(angle: number): void {
    if (this.stage.kind !== 'angle') return
    if (Math.abs(angle - this.stage.angle) < ANGLE_EPS) return
    this.stage = { ...this.stage, angle }
    if (this.typed === '') this._reportAngle()
    this._updatePreview()
  }

  private _reportAngle(): void {
    if (this.stage.kind !== 'angle') return
    this.onMeasurementCb(formatDegrees(this.stage.angle))
  }

  private _updatePreview(): void {
    if (this.stage.kind !== 'angle') return
    const { info, axisPoint, axisDir, angle, segments } = this.stage
    const key = `${angle.toFixed(5)}:${segments}`
    if (key === this.ghostKey && this.ghost !== null) return
    this.ghostKey = key
    if (info.profile.kind === 'face' && info.profile.instance !== undefined) {
      // No preview entry point for a component's own face: the commit still
      // goes through `revolve_in_instance`.
      return
    }
    try {
      const mesh = this.wasmScene.revolve_preview(
        revolveProfileJson(info.profile),
        revolveSpecJson(axisPoint, axisDir, angle, segments),
      )
      this._clearGhost()
      const ghost = buildMeshPreview(mesh, GHOST_COLOR)
      this.previewProblem = ''
      if (ghost !== null) {
        this.ghost = ghost
        this.preview.add(ghost)
      }
    } catch (err) {
      this._clearGhost()
      this.ghostKey = key
      const code = parseKernelErrorCode(err)
      this.previewProblem = kernelErrorMessage(code ?? 'Unknown', err instanceof Error ? err.message : String(err))
    }
  }

  // ------------------------------------------------------------------ commit

  private _commitCurrent(angle: number): void {
    if (this.stage.kind !== 'angle') return
    const { info, axisPoint, axisDir, segments } = this.stage
    const spec: RetypeSpec = { info, axisPoint, axisDir, angle, segments }
    try {
      const id = this._invoke(spec)
      this._afterCommit(id, spec)
    } catch (err) {
      this._toastRefusal(err)
    }
  }

  private _invoke(spec: RetypeSpec, replay = false): bigint {
    const { axisPoint, axisDir, angle, segments } = spec
    // A retype replays after an undo, which re-keys the profile's handles:
    // look at the same place again for the profile instead of trusting them.
    const info = replay ? this._refreshProfile(spec.info) : spec.info
    const specJson = revolveSpecJson(axisPoint, axisDir, angle, segments)
    if (info.profile.kind === 'face' && info.profile.instance !== undefined && this._activeInstance !== null) {
      return this.wasmScene.revolve_in_instance(
        this._activeInstance,
        info.profile.object,
        info.profile.face,
        specJson,
      )
    }
    return this.wasmScene.revolve(revolveProfileJson(info.profile), specJson)
  }

  private _refreshProfile(info: ProfileInfo): ProfileInfo {
    if (this._activeInstance !== null) return info
    try {
      const ray = rayAtProfile(info.ring, info.plane.normal)
      if (info.profile.kind === 'region') {
        const hit = pickRegionAt(this.wasmScene, ray, null)
        return hit === null
          ? info
          : { ...info, profile: { kind: 'region', sketch: hit.sketchHandle, region: hit.regionHandle } }
      }
      const hit = pickFaceAt(this.wasmScene, ray)
      return hit === null || hit.objectHandle !== info.profile.object || hit.instance !== undefined
        ? info
        : { ...info, profile: { ...info.profile, face: hit.faceHandle } }
    } catch {
      return info
    }
  }

  private _afterCommit(id: bigint, spec: RetypeSpec): void {
    this.stage = { kind: 'pick-profile' }
    this.typed = ''
    this.hovered = null
    this.axisProblem = ''
    this.previewProblem = ''
    this._clearAll()
    this.onMeasurementCb('')
    if (typeof this.wasmScene.history_generation === 'function') this.retype.arm(spec)
    this.onCommit(id)
  }

  private _toastRefusal(err: unknown): void {
    const code = parseKernelErrorCode(err)
    const raw = err instanceof Error ? err.message : String(err)
    this.onToast(kernelErrorMessage(code ?? 'Unknown', raw), code ?? undefined)
  }

  /** Redoes the last revolve with `change`; true when the redo committed. */
  private _retypeApply(change: { angle?: number; segments?: number }): boolean {
    const spec0 = this.retype.spec
    if (spec0 === null) return false
    const next: RetypeSpec = {
      ...spec0,
      angle: change.angle ?? spec0.angle,
      segments: change.segments ?? spec0.segments,
    }
    const result: { id: bigint | null } = { id: null }
    const run = (s: RetypeSpec): boolean => {
      try {
        result.id = this._invoke(s, true)
        return true
      } catch (err) {
        this._toastRefusal(err)
        return false
      }
    }
    const outcome = this.retype.apply(
      () => run(next),
      (s) => run(s),
      () => next,
    )
    if (outcome === 'stale') {
      this.onToast(retypeStaleMessage('revolve'))
      return false
    }
    if (result.id !== null) this.onCommit(result.id)
    return result.id !== null
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

  private _clearAll(): void {
    this._clearGhost()
    this.profileLine.clear()
    this.hoverLine.clear()
    this.axisLine.clear()
  }
}
