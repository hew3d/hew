/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * LoftTool — skin a solid through two or more shapes (true-curves.md §5.4):
 * a square at the bottom, a circle at the top, and the loft blends one into
 * the other; end on a point for a nose or a roof.
 *
 * Gesture:
 *   1. Click the shapes in order: drawn shapes (sketch regions) or solid
 *      faces. Each picked shape gets its number. Click one again to take it
 *      back out.
 *   2. To end at a point, click a point (an endpoint snap) once at least one
 *      shape is picked; that ends the picking.
 *   3. Alt-click a drawn line or curve to add a rail the loft follows (up to
 *      two). The rail is the run of lines through the clicked one up to a
 *      fork, so a rail drawn from a shape's corner in its own drawing is
 *      the rail alone. A rail decides which points of the shapes pair, so
 *      the start grips go away while one is picked; a rail the kernel
 *      refuses is not taken and a toast says why.
 *   4. Each shape shows a small grip where the matching starts. Drag it to
 *      another corner of that shape to turn the match.
 *   5. After two shapes a ghost shows the kernel's own result. Enter makes
 *      the loft; `8s` sets the steps along it; Esc takes back the last pick.
 *
 * The kernel owns the decisions (planar closed shapes, no crossing); an
 * open drawn chain is refused here, in plain words, since it is the one
 * thing the user can fix by drawing.
 */

import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import { sameRay, screenConstantWorldHalfFromWorldPerPixel } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { defaultFaceEligible, type FaceEligible } from './faceDraw'
import { clearPreview } from './transformPreview'
import { getLengthUnit } from '../settings/units'
import { rayPlaneIntersect, normalizeV3, type V3 } from '../viewport/geoHelpers'
import { pickRegionAt, pickFaceAt, regionRing, ringSegments, interiorPoint } from './profilePick'
import { pickEdgePath, pickEdgeRun, edgePathPolyline, polylineSegments } from './edgePathPick'
import { LineSlot } from './previewLines'
import { buildMeshPreview } from './meshPreview'
import { distSqToRay } from './boxGizmo'
import { cueColors, pathHoverColors } from './FollowMeTool'
import { editShapeBuffer, isBareModifier, isShapeInputKey, parseShapeToken } from './shapeInput'
import {
  loftSpecJson,
  nearestVertexIndex,
  parseStartPoints,
  type LoftRail,
  type LoftSource,
} from './loftMath'

export type OnLoftCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

const GHOST_COLOR = 0xee8800
const GRIP_SCREEN_PX = 10
const GRIP_PICK_PX_MULT = 2.2
const FALLBACK_GRIP_HALF_M = 0.02
const BASE_HINT =
  'Click profiles in order. Click a point to end at an apex. Alt-click a path to add a rail. Enter to loft.'
const OPEN_CHAIN = 'Loft needs closed shapes. Close this one first.'
const POINT_FIRST = 'Click the shapes first, then the point to end at.'
/** Refusals that belong to a rail rather than to the shapes. */
const RAIL_CODES = new Set(['RailMissesProfile', 'LoftTooManyRails', 'PathBranches', 'PathDisconnected', 'PathSegmentTooShort', 'EmptyPath'])

interface Picked {
  source: LoftSource
  /** The shape's outline, for the highlight and the label spot. Empty for an apex. */
  ring: V3[]
  /** Plane of the shape (point + unit normal), for the start-grip drag. */
  plane: { point: V3; normal: V3 } | null
}

type Mark = 'profile' | 'apex' | 'rail'

export class LoftTool implements Tool {
  readonly name = 'Loft'

  private picked: Picked[] = []
  private rails: (LoftRail & { points: number[] })[] = []
  /** Order the picks were made in, so Esc takes back the latest of any kind. */
  private marks: Mark[] = []
  private offsets: number[] = []
  private stations = 0
  private typed = ''

  /** Original vertices of each profile in oriented order from the automatic
   *  start (from `loft_start_points` with no offsets). */
  private baseStarts: V3[][] = []
  private gripMeshes: THREE.Mesh[] = []
  private labels: THREE.Sprite[] = []
  private worldPerPixel: ((dist: number) => number) | null = null
  private dragging: { k: number } | null = null

  private readonly outlines: LineSlot[] = []
  private readonly railLines: LineSlot[] = []
  private readonly hoverLine: LineSlot
  private ghost: THREE.Group | null = null
  private ghostKey = ''
  private problem = ''
  /** A one-line confirmation ("Stations set to 8.") shown until the next
   *  pointer move or key. `noteRay` is the ray it was set under: the
   *  Viewport re-sends that ray after a key, which is not a move. */
  private note = ''
  private noteRay: Ray | null = null
  private lastRay: Ray | null = null
  private hoverPoint = false
  private missNotified = false
  private altHeld = false

  private _editContext: EditContext = { kind: 'top' }
  private _faceEligible: FaceEligible | null = null

  constructor(
    private readonly wasmScene: WasmScene,
    private readonly preview: THREE.Group,
    private readonly onCommit: OnLoftCommit,
    private readonly onToast: OnToast,
    private readonly onMeasurementCb: OnMeasurement = () => { /* no-op */ },
  ) {
    this.hoverLine = new LineSlot(preview, 995)
  }

  // ------------------------------------------------------------- Tool plumbing

  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
    this.cancel()
  }

  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }

  /** The live Alt state at the click, read by the Viewport from the real
   *  pointer event (a tool's `onPointerDown` carries no modifiers). */
  setAltModifier(held: boolean): void {
    this.altHeld = held
  }

  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
  }

  capturingInput(): boolean {
    return this.picked.length > 0 || this.rails.length > 0
  }

  hasArmedGesture(): boolean {
    return this.capturingInput() || this.typed !== ''
  }

  capturesKey(key: string): boolean {
    if (!this.capturingInput()) return false
    if (key === 'Enter') return true
    if (this.typed === '') return key === 'Backspace' ? false : /^[0-9]$/.test(key)
    return isShapeInputKey(key)
  }

  onDocumentReset(): void {
    this.cancel()
  }

  statusHint(): string {
    // A problem outranks the confirmation note.
    if (this.problem !== '') return this.problem
    if (this.note !== '') return this.note
    if (this.dragging !== null) return 'Drag the grip to the corner where this shape should start matching. Release to set it.'
    if (this.hoverPoint) return 'Click this point to end the loft at an apex.'
    return BASE_HINT
  }

  cancel(): void {
    this.note = ''
    this.picked = []
    this.rails = []
    this.marks = []
    this.offsets = []
    this.stations = 0
    this.typed = ''
    this.baseStarts = []
    this.dragging = null
    this.problem = ''
    this.hoverPoint = false
    this.missNotified = false
    this._clearVisuals()
    this.onMeasurementCb('')
    clearPreview(this.preview)
    this.worldPerPixel = null
  }

  /** Per-frame: keep the start grips and number labels a constant size on screen. */
  updateGripScale(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    this.worldPerPixel = worldPerPixel
    for (const m of this.gripMeshes) {
      const dist = camera.position.distanceTo(m.position)
      m.scale.setScalar(screenConstantWorldHalfFromWorldPerPixel(GRIP_SCREEN_PX / 2, worldPerPixel(dist), 1e-5))
    }
    for (const s of this.labels) {
      const dist = camera.position.distanceTo(s.position)
      const h = screenConstantWorldHalfFromWorldPerPixel(11, worldPerPixel(dist), 1e-5)
      s.scale.set(h * 2, h * 2, 1)
    }
  }

  // ------------------------------------------------------------------ pointer

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (!sameRay(this.noteRay, ray)) this.note = ''
    this.lastRay = ray
    if (this.dragging !== null) {
      this._dragGrip(ray)
      return
    }
    this.hoverPoint = false
    const profile = this._profileAt(ray)
    if (profile !== null) {
      this.hoverLine.set(ringSegments(profile.ring), pathHoverColors().hover, this._keyOf(profile.source))
      return
    }
    this.hoverLine.clear()
    if (this._canEndAtPoint() && this._vertexSnap(snap) !== null) this.hoverPoint = true
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    this.note = ''
    const grip = this._gripAt(ray)
    if (grip !== null) {
      this.dragging = { k: grip }
      this.hoverLine.clear()
      return
    }

    // Alt-click on a drawn line or curve: a rail.
    if (this.altHeld) {
      this._addRail(ray)
      return
    }

    const profile = this._profileAt(ray)
    if (profile !== null) {
      this.missNotified = false
      this._togglePick(profile)
      return
    }

    // A point before any shape: the shapes come first.
    if (this.picked.length === 0 && this._vertexSnap(snap) !== null) {
      this._notifyMiss(POINT_FIRST)
      return
    }

    // A vertex with no shape under it ends the loft at an apex.
    if (this._canEndAtPoint()) {
      const point = this._vertexSnap(snap)
      if (point !== null) {
        this.picked.push({ source: { kind: 'vertex', point }, ring: [], plane: null })
        this.marks.push('apex')
        this.offsets.push(0)
        this._dropRefusedRails()
        this._refresh()
        return
      }
    }

    // A drawn chain that is not a closed shape: say so.
    try {
      if (pickEdgePath(this.wasmScene, ray) !== null) {
        this._notifyMiss(OPEN_CHAIN)
        return
      }
    } catch {
      // fall through to the generic hint
    }
    this._notifyMiss(BASE_HINT)
  }

  onPointerUp(): void {
    if (this.dragging === null) return
    this.dragging = null
    this._refresh()
  }

  // ----------------------------------------------------------------- keyboard

  onKey(ev: KeyboardEvent): void {
    if (!isBareModifier(ev.key)) this.note = ''
    if (ev.key === 'Escape') {
      // A half-typed count goes first; only then does Esc take a pick back.
      if (this.typed !== '') {
        this.typed = ''
        this.onMeasurementCb('')
        return
      }
      this._takeBackLast()
      return
    }
    if (ev.key === 'Enter') {
      const token = parseShapeToken(this.typed)
      if (token !== null && token.kind === 'count') {
        this.stations = token.n
        this.typed = ''
        this.onMeasurementCb('')
        this.note = `Stations set to ${token.n}.`
        this.noteRay = this.lastRay
        this._refresh()
        return
      }
      if (this.typed !== '') return
      this._commit()
      return
    }
    if (this.typed === '' && !/^[0-9]$/.test(ev.key)) return
    if (isShapeInputKey(ev.key)) {
      this.typed = editShapeBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this.typed)
    }
  }

  private _takeBackLast(): void {
    const mark = this.marks.pop()
    if (mark === undefined) {
      this.cancel()
      return
    }
    if (mark === 'rail') this.rails.pop()
    else {
      this.picked.pop()
      this.offsets.pop()
    }
    this.problem = ''
    this._refresh()
  }

  // ---------------------------------------------------------------- picking

  private _canEndAtPoint(): boolean {
    const last = this.picked[this.picked.length - 1]
    return this.picked.length >= 1 && last.source.kind !== 'vertex'
  }

  private _vertexSnap(snap: Snap | null): V3 | null {
    if (snap === null || snap.kind !== 'endpoint') return null
    return [snap.x, snap.y, snap.z]
  }

  private _faceOk(object: bigint, instance: bigint | undefined): boolean {
    const active = this._activeInstance
    if (active !== null) return instance === active
    if (instance !== undefined) return false
    return this._faceEligible !== null
      ? this._faceEligible(object, instance)
      : defaultFaceEligible(this.wasmScene, this._editContext.kind === 'object' ? this._editContext.id : null, object, instance)
  }

  /** The shape under the ray: a drawn region first, then an eligible face. */
  private _profileAt(ray: Ray): Picked | null {
    try {
      const region = pickRegionAt(this.wasmScene, ray, this._activeInstance)
      if (region !== null) {
        const ring = regionRing(this.wasmScene, region.sketchHandle, region.regionHandle)
        const pl = this.wasmScene.sketch_plane(region.sketchHandle)
        if (ring.length < 3) return null
        return {
          source: { kind: 'region', sketch: region.sketchHandle, region: region.regionHandle },
          ring,
          plane: pl !== undefined && pl.length >= 6 ? { point: [pl[0], pl[1], pl[2]], normal: [pl[3], pl[4], pl[5]] } : null,
        }
      }
      const face = pickFaceAt(this.wasmScene, ray)
      if (face === null || !this._faceOk(face.objectHandle, face.instance)) return null
      const loop = this.wasmScene.face_boundary(face.objectHandle, face.faceHandle)
      const pl = this.wasmScene.face_plane(face.objectHandle, face.faceHandle)
      const ring: V3[] = []
      for (let i = 0; i + 2 < loop.length; i += 3) ring.push([loop[i], loop[i + 1], loop[i + 2]])
      if (ring.length < 3) return null
      return {
        source: { kind: 'face', object: face.objectHandle, face: face.faceHandle },
        ring,
        plane: pl !== undefined && pl.length >= 6 ? { point: [pl[0], pl[1], pl[2]], normal: [pl[3], pl[4], pl[5]] } : null,
      }
    } catch {
      return null
    }
  }

  private _keyOf(s: LoftSource): string {
    if (s.kind === 'region') return `r:${s.sketch}:${s.region}`
    if (s.kind === 'face') return `f:${s.object}:${s.face}`
    return `v:${s.point.join(',')}`
  }

  /** Whether a picked shape has a hole: the kernel refuses such a shape at
   *  Enter, so it is refused at the pick instead. This covers a drawn region
   *  (free or on a solid's face) and a solid's face (a box with a hole
   *  through it). A shape that cannot be read leaves the decision to the
   *  kernel. */
  private _regionHasHole(source: LoftSource): boolean {
    try {
      if (source.kind === 'face') return this.wasmScene.face_hole_count(source.object, source.face) > 0
      if (source.kind === 'region') return this.wasmScene.region_hole_count(source.sketch, source.region) > 0
      return false
    } catch {
      return false
    }
  }

  private _togglePick(profile: Picked): void {
    const key = this._keyOf(profile.source)
    const at = this.picked.findIndex((p) => this._keyOf(p.source) === key)
    if (at >= 0) {
      // Clicking a picked shape again takes it back out.
      this.picked.splice(at, 1)
      this.offsets.splice(at, 1)
      this._dropMark('profile', at)
      this.problem = ''
      this._refresh()
      return
    }
    if (this.picked.some((p) => p.source.kind === 'vertex')) {
      this.onToast('The loft already ends at a point. Press Esc to take it back first.')
      return
    }
    if (this._regionHasHole(profile.source)) {
      this.onToast(kernelErrorMessage('LoftProfileHasHole', ''), 'LoftProfileHasHole')
      return
    }
    this.picked.push(profile)
    this.marks.push('profile')
    this.offsets.push(0)
    this._dropRefusedRails()
    this._refresh()
  }

  /** A rail added before the shapes can turn out to miss a shape picked
   *  later. Take the rail back off with a toast that says why, so the marks,
   *  the rails, and the status line stay in step. */
  private _dropRefusedRails(): void {
    if (this.rails.length === 0 || this.picked.length < 2) return
    const all = this._railRefusalFor(this.rails)
    if (all === null) return
    let at = -1
    for (let i = this.rails.length - 1; i >= 0; i--) {
      if (this._railRefusalFor([this.rails[i]]) !== null) {
        at = i
        break
      }
    }
    if (at < 0) at = this.rails.length - 1
    this.rails.splice(at, 1)
    // Drop the at-th 'rail' mark.
    this._dropMark('rail', at)
    this.onToast(`That path was taken off the loft. ${all.message}`, all.code)
  }

  /** Drop the `n`-th 'profile' entry from the pick-order stack. */
  private _dropMark(kind: Mark, n: number): void {
    let seen = -1
    for (let i = 0; i < this.marks.length; i++) {
      if (this.marks[i] === kind && ++seen === n) {
        this.marks.splice(i, 1)
        return
      }
    }
  }

  private _addRail(ray: Ray): void {
    // The run through the clicked edge, not its whole drawing: a rail drawn
    // from a rectangle's corner in the rectangle's own sketch forks there,
    // and only the rail is meant.
    let path: ReturnType<typeof pickEdgeRun> = null
    try {
      path = pickEdgeRun(this.wasmScene, ray)
    } catch {
      path = null
    }
    if (path === null) {
      this._notifyMiss('Alt-click a drawn line or curve to add a rail.')
      return
    }
    if (this.rails.length >= 2) {
      this.onToast('A loft can follow at most two paths.')
      return
    }
    const poly = edgePathPolyline(this.wasmScene, path.sketchHandle, path.edgeHandles)
    const rail = {
      sketch: path.sketchHandle,
      edges: path.edgeHandles,
      points: poly === null ? [] : polylineSegments(poly),
    }
    // With the shapes to loft between already picked, a rail the kernel
    // refuses (it misses a shape, or pairs points another rail does not)
    // is not taken: the toast names why, and the rail count stays honest.
    const refusal = this._railRefusal(rail)
    if (refusal !== null) {
      this.onToast(refusal.message, refusal.code)
      return
    }
    this.rails.push(rail)
    this.offsets = this.picked.map(() => 0)
    this.marks.push('rail')
    this.missNotified = false
    this._refresh()
  }

  /** The kernel's refusal of the loft with `rail` added, when that refusal
   *  is the rail's (it misses a shape, or its path cannot be read); null
   *  when the rail is fine or the shapes are not picked yet. */
  private _railRefusal(rail: LoftRail): { message: string; code: string } | null {
    return this._railRefusalFor([...this.rails, rail])
  }

  private _railRefusalFor(rails: readonly LoftRail[]): { message: string; code: string } | null {
    if (this.picked.length < 2) return null
    const json = loftSpecJson(
      this.picked.map((p) => p.source),
      rails,
      this.picked.map(() => 0),
      this.stations,
    )
    try {
      this.wasmScene.loft_preview(json)
      return null
    } catch (err) {
      const code = parseKernelErrorCode(err)
      if (code === null || !RAIL_CODES.has(code)) return null
      return { message: kernelErrorMessage(code, err instanceof Error ? err.message : String(err)), code }
    }
  }

  private _notifyMiss(message: string): void {
    if (this.missNotified) return
    this.missNotified = true
    this.onToast(message)
  }

  // ------------------------------------------------------------ start grips

  private _specJson(offsets: readonly number[] = this._effectiveOffsets()): string {
    return loftSpecJson(
      this.picked.map((p) => p.source),
      this.rails,
      offsets,
      this.stations,
    )
  }

  /** With a rail the rail decides which points pair, so start offsets
   *  dragged earlier are ignored rather than applied alongside it. */
  private _effectiveOffsets(): number[] {
    return this.rails.length > 0 ? this.picked.map(() => 0) : this.offsets
  }

  private _gripPosition(k: number): V3 | null {
    const list = this.baseStarts[k]
    if (list === undefined || list.length === 0) return null
    // The kernel wraps an offset round the candidates (a start offset past
    // the last turns on from the first), so the grip does too: an offset
    // kept from picks with more candidates (a shape taken out, or every
    // shape now having the same number of corners) marks where the loft
    // really starts.
    const n = list.length
    return list[(((this.offsets[k] ?? 0) % n) + n) % n]
  }

  private _gripAt(ray: Ray): number | null {
    let best: number | null = null
    let bestRatio = Infinity
    for (let k = 0; k < this.picked.length; k++) {
      const pos = this._gripPosition(k)
      if (pos === null || this.picked[k].source.kind === 'vertex') continue
      const dx = pos[0] - ray.origin[0], dy = pos[1] - ray.origin[1], dz = pos[2] - ray.origin[2]
      const dist = Math.hypot(dx, dy, dz)
      const half = this.worldPerPixel === null
        ? FALLBACK_GRIP_HALF_M
        : screenConstantWorldHalfFromWorldPerPixel(GRIP_SCREEN_PX / 2, this.worldPerPixel(dist), 1e-5)
      const tol = half * GRIP_PICK_PX_MULT
      const ratio = distSqToRay(ray, pos) / (tol * tol)
      if (ratio < bestRatio) {
        bestRatio = ratio
        best = k
      }
    }
    return best !== null && bestRatio <= 1 ? best : null
  }

  private _dragGrip(ray: Ray): void {
    if (this.dragging === null) return
    const { k } = this.dragging
    const pick = this.picked[k]
    const list = this.baseStarts[k]
    if (pick === undefined || pick.plane === null || list === undefined) return
    const n = normalizeV3(pick.plane.normal)
    if (n === null) return
    const hit = rayPlaneIntersectAny(ray, pick.plane.point, n)
    if (hit === null) return
    const idx = nearestVertexIndex(list, hit, n)
    if (idx < 0 || idx === this.offsets[k]) return
    this.offsets[k] = idx
    this._refresh()
  }

  // ------------------------------------------------------- refresh and ghost

  /** Redraw everything the picks imply: outlines, labels, rails, start
   *  markers, and the ghost. */
  private _refresh(): void {
    this._clearVisuals()
    const colors = pathHoverColors()
    this.picked.forEach((p, k) => {
      if (p.ring.length === 0) return
      const slot = new LineSlot(this.preview, 996)
      slot.set(ringSegments(p.ring), colors.path)
      this.outlines.push(slot)
      this._addLabel(k, p)
    })
    this.rails.forEach((r) => {
      const slot = new LineSlot(this.preview, 997)
      slot.set(r.points, cueColors().orient)
      this.railLines.push(slot)
    })
    this._updateStartsAndGhost()
  }

  private _addLabel(k: number, p: Picked): void {
    const normal = p.plane?.normal ?? [0, 0, 1]
    const at = interiorPoint(p.ring, normal)
    const sprite = makeNumberSprite(k + 1)
    if (sprite === null) return
    sprite.position.set(at[0], at[1], at[2])
    this.preview.add(sprite)
    this.labels.push(sprite)
  }

  private _updateStartsAndGhost(): void {
    for (const m of this.gripMeshes) {
      this.preview.remove(m)
      m.geometry.dispose()
      ;(m.material as THREE.Material).dispose()
    }
    this.gripMeshes = []
    this.baseStarts = []
    this.problem = ''
    if (this.picked.length < 2) {
      this._clearGhost()
      return
    }
    // With a rail the rail decides which points pair (D-232), so no start
    // grips: dragging one would change nothing.
    if (this.rails.length > 0) {
      this._previewGhost()
      return
    }
    // The start grips ride the automatic starts (offsets all zero).
    try {
      const flat = this.wasmScene.loft_start_points(this._specJson(this.picked.map(() => 0)))
      this.baseStarts = parseStartPoints(flat)
    } catch {
      this.baseStarts = []
    }
    this.baseStarts.forEach((_list, k) => {
      const pos = this._gripPosition(k)
      if (pos === null || this.picked[k]?.source.kind === 'vertex') return
      const mesh = new THREE.Mesh(
        new THREE.BoxGeometry(2, 2, 2),
        new THREE.MeshBasicMaterial({ color: 0x33dd77, depthTest: false }),
      )
      mesh.position.set(pos[0], pos[1], pos[2])
      mesh.scale.setScalar(FALLBACK_GRIP_HALF_M)
      mesh.renderOrder = 998
      this.preview.add(mesh)
      this.gripMeshes.push(mesh)
    })
    this._previewGhost()
  }

  private _previewGhost(): void {
    const json = this._specJson()
    if (json === this.ghostKey && this.ghost !== null) return
    try {
      const mesh = this.wasmScene.loft_preview(json)
      this._clearGhost()
      const ghost = buildMeshPreview(mesh, GHOST_COLOR)
      if (ghost !== null) {
        this.ghost = ghost
        this.preview.add(ghost)
      }
      this.ghostKey = json
      this.problem = ''
    } catch (err) {
      this._clearGhost()
      const code = parseKernelErrorCode(err)
      this.problem = kernelErrorMessage(code ?? 'Unknown', err instanceof Error ? err.message : String(err))
    }
  }

  // ------------------------------------------------------------------ commit

  private _commit(): void {
    if (this.picked.length < 2) {
      this.onToast('Pick at least two shapes (or a shape and a point) to loft between.')
      return
    }
    try {
      const id = this.wasmScene.loft(this._specJson())
      this.cancel()
      this.onCommit(id)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', err instanceof Error ? err.message : String(err)), code ?? undefined)
    }
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

  private _clearVisuals(): void {
    this._clearGhost()
    this.hoverLine.clear()
    for (const s of this.outlines) s.clear()
    this.outlines.length = 0
    for (const s of this.railLines) s.clear()
    this.railLines.length = 0
    for (const m of this.gripMeshes) {
      this.preview.remove(m)
      m.geometry.dispose()
      ;(m.material as THREE.Material).dispose()
    }
    this.gripMeshes = []
    for (const s of this.labels) {
      this.preview.remove(s)
      s.material.map?.dispose()
      s.material.dispose()
    }
    this.labels = []
  }
}

/** The ray's hit on a plane, also when the plane lies behind the eye. */
function rayPlaneIntersectAny(ray: Ray, point: V3, normal: V3): V3 | null {
  const fwd = rayPlaneIntersect(ray.origin as V3, ray.direction as V3, point, normal)
  if (fwd !== null) return fwd
  const denom = ray.direction[0] * normal[0] + ray.direction[1] * normal[1] + ray.direction[2] * normal[2]
  if (Math.abs(denom) < 1e-10) return null
  const t =
    ((point[0] - ray.origin[0]) * normal[0] +
      (point[1] - ray.origin[1]) * normal[1] +
      (point[2] - ray.origin[2]) * normal[2]) /
    denom
  return [ray.origin[0] + t * ray.direction[0], ray.origin[1] + t * ray.direction[1], ray.origin[2] + t * ray.direction[2]]
}

/** A screen-facing number label, or null where no 2D canvas exists (tests). */
function makeNumberSprite(n: number): THREE.Sprite | null {
  if (typeof document === 'undefined') return null
  const canvas = document.createElement('canvas')
  canvas.width = 64
  canvas.height = 64
  const ctx = canvas.getContext('2d')
  if (ctx === null) return null
  ctx.fillStyle = '#1b2a41'
  ctx.beginPath()
  ctx.arc(32, 32, 28, 0, Math.PI * 2)
  ctx.fill()
  ctx.fillStyle = '#ffffff'
  ctx.font = 'bold 34px sans-serif'
  ctx.textAlign = 'center'
  ctx.textBaseline = 'middle'
  ctx.fillText(String(n), 32, 34)
  const tex = new THREE.CanvasTexture(canvas)
  const mat = new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true })
  const sprite = new THREE.Sprite(mat)
  sprite.renderOrder = 999
  return sprite
}
