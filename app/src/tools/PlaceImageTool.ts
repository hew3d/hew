/**
 * PlaceImageTool — Place Image: put a reference picture (PNG/JPEG) on a
 * face or the ground with two clicks, its proportions kept.
 *
 * Armed by File ▸ Import Image… or a picture dropped on the viewport, with
 * the picture's bytes and its pixel size (`imageInfo.ts`). The gesture:
 *
 *   1. Idle: a ghost of the picture, 1 m wide, rides the cursor with its
 *      lower-left corner at the cursor, on the plane a click would use —
 *      the face under the cursor (any face, grouped or not: a picture is
 *      placed as its own component, never drawn into the face), else the
 *      ground. An arrow key locks the plane's normal to a drawing axis
 *      through the clicked point instead (the draw tools' idle plane
 *      lock); the same arrow, or Esc, unlocks.
 *   2. First click fixes one corner on that plane. Moving sizes the
 *      picture: its width follows the cursor along the picture's bottom
 *      edge, its height follows from its proportions, and it flips to
 *      whichever side of the corner the cursor is on (Rectangle's flip) —
 *      without ever mirroring the picture. The Measurements box shows the
 *      width; a typed width sizes the ghost at once (on the cursor's
 *      side), and Enter or a click commits at that width.
 *   3. Second click (or Enter) commits ONE `Scene.place_image` — material,
 *      definition and instance as one undo step — and the tool retires to
 *      Select with the new instance selected (the LibraryPlaceTool
 *      one-shot convention).
 *   4. Esc after the first click backs up to step 1; before it, Esc clears
 *      an arrow lock, and with no lock cancels the placement.
 *
 * The picture always reads upright: its up is the drawing axes' blue axis
 * laid onto the plane (the green axis on a level plane), and it faces the
 * side of the plane the camera is on.
 */

import * as THREE from 'three'
import type { Tool, Snap, SnapConstraint } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { rayPlaneIntersect } from '../viewport/geoHelpers'
import { axisDrawPlane, inferredAxis, inferredAxisPlane, type CameraView } from './drawPlane'
import { cameraView } from './viewPlaneInference'
import { getDrawingAxes } from './drawingAxes'
import { FacePickCache, worldFacePlane, snapOnObjectBoundary } from './faceDraw'
import { t } from '@lingui/core/macro'
import { formatNumber } from '../i18n/format'
import { nextIdlePlaneLock, editLengthBuffer, isLengthInputKey, axisLockColorName } from './moveInput'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'
import type { ImageFormatCode } from './imageInfo'

/** Everything the tool needs about the picture. */
export interface ImagePlacement {
  bytes: Uint8Array
  format: ImageFormatCode
  /** The definition's name (the file's name without its extension). */
  name: string
  /** Shown size in pixels — the proportions the placed face keeps. */
  pixelWidth: number
  pixelHeight: number
  /** The picture for the ghost (owned by the tool from here on), or null
   *  for an outline-only ghost. */
  texture: THREE.Texture | null
}

/** What one placement made (`Scene.place_image`'s result). */
export interface PlacedImage {
  definition: bigint
  instance: bigint
  width: number
  height: number
}

export type OnImagePlaced = (result: PlacedImage) => void
/** Fired exactly once when the tool retires — see LibraryPlaceTool's
 *  `OnDone` for `byUser`. */
export type OnImageDone = (placed: boolean, byUser: boolean) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** The ghost's width before the first click, in meters. */
export const PROVISIONAL_WIDTH_M = 1

/** Below this the cursor has not left the corner (no size, no side). */
const SIZE_EPS = 1e-6

/** The shortest side a placed picture may have, in meters: the kernel's
 *  own floor (`image_import::MIN_SIDE`), below which it cannot build the
 *  picture's face. The tool refuses a smaller picture with the kernel's
 *  `ImageTooSmall` copy before asking, rather than letting it reach the
 *  kernel at all. */
export const PICTURE_MIN_SIDE_M = 1e-6

/** The longest side a placed picture may have, in meters
 *  (`image_import::MAX_SIDE`, 1,000 km). A width dragged or typed past it
 *  is refused with its own `ImageTooBig` toast before it reaches the
 *  kernel. */
export const PICTURE_MAX_SIDE_M = 1e6

/** A length in meters as plain text for the too-big toast: grouped digits
 *  up to a quadrillion, scientific notation past that (so a typed `1e300`
 *  does not print 300 digits). */
function metersText(m: number): string {
  if (m >= 1e15) return m.toExponential(2).replace('e+', 'e')
  return formatNumber(m, { maxFractionDigits: 3 })
}

/** The plane a picture lies on, with the picture's own axes: `u` along its
 *  bottom edge (left to right), `v` toward its top, `normal` = u × v,
 *  facing the camera. */
export interface PictureFrame {
  point: V3
  normal: V3
  u: V3
  v: V3
}

type Stage =
  | { kind: 'idle' }
  | { kind: 'anchored'; frame: PictureFrame; corner: V3 }

const dot = (a: V3, b: V3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
const scale = (a: V3, s: number): V3 => [a[0] * s, a[1] * s, a[2] * s]
const add = (a: V3, b: V3): V3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
function unit(a: V3): V3 | null {
  const l = Math.hypot(a[0], a[1], a[2])
  return l < 1e-12 ? null : [a[0] / l, a[1] / l, a[2] / l]
}

/**
 * The picture's frame on the plane through `point` with normal `normal`,
 * seen along `viewDir`: the normal turned toward the viewer, up = `up`
 * laid onto the plane (or `fallbackUp` when `up` is square to it), and the
 * bottom edge running left to right as seen. Exported for tests.
 */
export function pictureFrame(point: V3, normal: V3, viewDir: V3, up: V3, fallbackUp: V3): PictureFrame | null {
  let n = unit(normal)
  if (n === null) return null
  if (dot(n, viewDir) > 0) n = scale(n, -1)
  const onPlane = (w: V3) => unit(sub(w, scale(n as V3, dot(w, n as V3))))
  const v = onPlane(up) ?? onPlane(fallbackUp)
  if (v === null) return null
  const u = unit(cross(v, n))
  if (u === null) return null
  return { point, normal: n, u, v }
}

/**
 * The placed picture's lower-left corner and size, from the first corner
 * and the cursor (both on `frame`'s plane): width = the cursor's distance
 * along `u`, height from `aspect` (width / height), on whichever side of
 * the corner the cursor is (`signs`, when given, overrides it — a typed
 * width keeps the side the cursor last showed). Null for no width yet.
 * Exported for tests.
 */
export function pictureRect(
  frame: PictureFrame,
  corner: V3,
  cursor: V3,
  aspect: number,
  typedWidth: number | null = null,
): { origin: V3; width: number; height: number; su: 1 | -1; sv: 1 | -1 } | null {
  const d = sub(cursor, corner)
  const du = dot(d, frame.u)
  const dv = dot(d, frame.v)
  const width = typedWidth ?? Math.abs(du)
  // The corner test is for the cursor only: a typed width is the size
  // asked for, however small (the tool refuses one under the minimum).
  const floor = typedWidth === null ? SIZE_EPS : 0
  if (!(width > floor) || !Number.isFinite(width)) return null
  const height = width / aspect
  const su: 1 | -1 = du < -SIZE_EPS ? -1 : 1
  const sv: 1 | -1 = dv < -SIZE_EPS ? -1 : 1
  const origin = add(add(corner, scale(frame.u, su < 0 ? -width : 0)), scale(frame.v, sv < 0 ? -height : 0))
  return { origin, width, height, su, sv }
}

export class PlaceImageTool implements Tool {
  /** Idle over empty space, the plane follows the view (`Tool.followsViewPlane`). */
  readonly followsViewPlane = true
  readonly name = 'PlaceImage'

  private wasmScene: WasmScene
  private preview: THREE.Group
  private placement: ImagePlacement
  private onPlaced: OnImagePlaced
  private onDone: OnImageDone
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  private stage: Stage = { kind: 'idle' }
  private idlePlaneLock: 0 | 1 | 2 | null = null
  /** The camera (forward and eye) from `updateCamera`: over empty space the
   *  picture goes on the axis plane that faces the camera (view-plane
   *  inference). */
  private view: CameraView | null = null
  /** The vertical axis (0 red, 1 green) the last idle hover inferred, or
   *  null when the ground, a face or a lock decided. */
  private inferredAxis: 0 | 1 | null = null
  private typed = ''
  private lastCursor: V3 | null = null
  private finished = false
  private escPressed = false
  private readonly pickCache = new FacePickCache()

  private ghost: THREE.Group | null = null
  private ghostQuad: THREE.Mesh | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    placement: ImagePlacement,
    onPlaced: OnImagePlaced,
    onDone: OnImageDone,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.placement = placement
    this.onPlaced = onPlaced
    this.onDone = onDone
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  /** width / height of the picture as shown. */
  private get aspect(): number {
    return this.placement.pixelWidth / this.placement.pixelHeight
  }

  updateCamera(camera: THREE.Camera): void {
    this.view = cameraView(camera)
  }

  statusHint(): string {
    if (this.stage.kind === 'anchored') {
      return t`Click the opposite corner — or type the width and press Enter.`
    }
    if (this.idlePlaneLock !== null) {
      const axis = axisLockColorName(this.idlePlaneLock)
      return t`Locked to the ${axis} plane — click the first corner; same arrow or Esc unlocks.`
    }
    const name = this.placement.name
    if (this.inferredAxis !== null) {
      const axis = axisLockColorName(this.inferredAxis)
      return t`Facing the ${axis} plane — click the first corner of "${name}" to place it there; ↑ for the ground. Esc cancels.`
    }
    return t`Click the first corner of "${name}" — on the ground or any face. Esc cancels.`
  }

  /** Armed until it places or is cancelled: Escape belongs to the
   *  placement, never to an open edit context underneath it. */
  hasArmedGesture(): boolean {
    return !this.finished
  }

  /** Typed width entry, once the first corner is down. */
  capturingInput(): boolean {
    return this.stage.kind === 'anchored'
  }

  /** Snaps stay on the picture's plane once its first corner is down.
   *  Idle and unlocked, a miss over empty space lands on the axis plane the
   *  view faces (`fallbackPlane` only — never sent to the kernel, so any
   *  geometry still snaps), matching where `idleFrame` places the picture. */
  snapConstraint(): SnapConstraint | null {
    if (this.stage.kind === 'anchored') {
      return { constraintPlane: { point: this.stage.frame.point, normal: this.stage.frame.normal } }
    }
    if (this.idlePlaneLock !== null || this.view === null) return null
    const inferred = inferredAxisPlane(getDrawingAxes(this.wasmScene), this.view)
    if (inferred.ground) return null
    return { fallbackPlane: { point: inferred.origin, normal: inferred.normal } }
  }

  /** The frame a click at this event would place the picture on, and the
   *  point on it under the cursor. */
  private idleFrame(snap: Snap | null, ray: Ray): { frame: PictureFrame; point: V3 } | null {
    this.inferredAxis = null
    const axes = getDrawingAxes(this.wasmScene)
    const frameAt = (point: V3, normal: V3) => {
      const f = pictureFrame(point, normal, ray.direction, axes.z, axes.y)
      return f === null ? null : { frame: f, point }
    }
    const snapPoint: V3 | null = snap !== null ? [snap.x, snap.y, snap.z] : null
    if (this.idlePlaneLock !== null) {
      // The lock skips the face pick below, yet the boundary pick's hover
      // memory (view-plane design §4.2 rule 2) must keep following the
      // cursor for when the lock is released at a corner; the pick itself
      // is discarded.
      if (!snapOnObjectBoundary(snap)) this.pickCache.pickFor(this.wasmScene, ray, () => true)
      const ground = rayPlaneIntersect(ray.origin, ray.direction, [0, 0, 0], [0, 0, 1])
      const through = snapPoint ?? ground
      if (through === null) return null
      return frameAt(through, axisDrawPlane(this.idlePlaneLock, through, axes).normal)
    }
    const face = this.facePlaneAt(snap, ray)
    if (face !== null) {
      const hit = snapPoint !== null && Math.abs(dot(sub(snapPoint, face.point), face.normal)) < 1e-6
        ? snapPoint
        : rayPlaneIntersect(ray.origin, ray.direction, face.point, face.normal)
      if (hit !== null) return frameAt(hit, face.normal)
    }
    // Over empty space: the axis plane that faces the camera, the ground
    // unless the view looks squarely at a wall (view-plane inference).
    if (this.view !== null) {
      const axis = inferredAxis(axes, this.view)
      const inferred = inferredAxisPlane(axes, this.view)
      if (!inferred.ground) {
        const onPlane = snapPoint !== null && Math.abs(dot(sub(snapPoint, inferred.origin), inferred.normal)) < 1e-6
        const hit = onPlane ? snapPoint : rayPlaneIntersect(ray.origin, ray.direction, inferred.origin, inferred.normal)
        if (hit === null) return null
        this.inferredAxis = axis === 2 ? null : axis
        return frameAt(hit, inferred.normal)
      }
    }
    const groundHit = snapPoint !== null && Math.abs(snapPoint[2]) < 1e-9
      ? snapPoint
      : rayPlaneIntersect(ray.origin, ray.direction, [0, 0, 0], [0, 0, 1])
    if (groundHit === null) return null
    return frameAt(groundHit, [0, 0, 1])
  }

  /** The world plane of the face under the cursor — any face, each mapped
   *  through the instance it was struck through. On an edge or corner the
   *  plain pick misses, so the ranked boundary pick answers. */
  private facePlaneAt(snap: Snap | null, ray: Ray): { point: V3; normal: V3 } | null {
    if (snapOnObjectBoundary(snap)) {
      const through = this.pickCache.faceThrough(
        this.wasmScene, ray, () => false, null, [[snap.x, snap.y, snap.z]], true,
      )
      if (through !== null) return { point: through.point, normal: through.normal }
    }
    this.pickCache.pickFor(this.wasmScene, ray, () => true)
    const raw = this.pickCache.rawPickFor(ray)
    if (raw === null) return null
    return worldFacePlane(this.wasmScene, raw.object, raw.face, raw.instance ?? null)
  }

  /** The cursor on the anchored plane: the (plane-constrained) snap, held
   *  to the plane, else ray ∩ plane. */
  private anchoredCursor(snap: Snap | null, ray: Ray, frame: PictureFrame): V3 | null {
    const p: V3 | null = snap !== null
      ? [snap.x, snap.y, snap.z]
      : rayPlaneIntersect(ray.origin, ray.direction, frame.point, frame.normal)
    if (p === null) return null
    return sub(p, scale(frame.normal, dot(sub(p, frame.point), frame.normal)))
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this.finished) return
    if (this.stage.kind === 'idle') {
      const at = this.idleFrame(snap, ray)
      if (at === null) return
      this.drawGhost(at.frame, at.point, PROVISIONAL_WIDTH_M, PROVISIONAL_WIDTH_M / this.aspect)
      return
    }
    const { frame, corner } = this.stage
    const cursor = this.anchoredCursor(snap, ray, frame)
    if (cursor === null) return
    this.lastCursor = cursor
    this.redrawAnchored(frame, corner, cursor)
  }

  /** The width a typed buffer stands for, or null for none (empty, or not
   *  yet a length: "1." or a lone unit mid-typing). A typed length under
   *  the minimum, zero included, is still the width asked for: a click or
   *  Enter refuses it (`refuseBadSize`), never falls back to the cursor. */
  private typedWidth(): number | null {
    if (this.typed === '') return null
    const meters = parseLengthToMeters(this.typed)
    if (meters === null || !Number.isFinite(meters)) return null
    return Math.abs(meters)
  }

  /** Whether a picture `width` wide is outside what the kernel can place:
   *  either side under `PICTURE_MIN_SIDE_M` (the `ImageTooSmall` toast) or
   *  over `PICTURE_MAX_SIDE_M` (the `ImageTooBig` toast, naming the size
   *  asked for). Keeps the tool armed and any typed buffer. */
  private refuseBadSize(width: number): boolean {
    const height = width / this.aspect
    if (width > PICTURE_MAX_SIDE_M || height > PICTURE_MAX_SIDE_M) {
      const w = metersText(width)
      const h = metersText(height)
      const max = metersText(PICTURE_MAX_SIDE_M)
      this.onToast(
        t`The picture would be ${w} × ${h} m, and each side must be at most ${max} m. Drag it out smaller or type a smaller width.`,
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code
        'ImageTooBig',
      )
      return true
    }
    if (width >= PICTURE_MIN_SIDE_M && height >= PICTURE_MIN_SIDE_M) return false
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code
    this.onToast(kernelErrorMessage('ImageTooSmall', ''), 'ImageTooSmall')
    return true
  }

  /** The anchored ghost and readout at `cursor`: sized by a typed width
   *  while one is pending (so what is drawn is what a click or Enter
   *  places), else by the cursor. */
  private redrawAnchored(frame: PictureFrame, corner: V3, cursor: V3): void {
    const rect = pictureRect(frame, corner, cursor, this.aspect, this.typedWidth())
    if (rect === null) this.hideGhost()
    else this.drawGhost(frame, rect.origin, rect.width, rect.height)
    // The buffer owns the readout while it holds anything.
    if (this.typed !== '') this.onMeasurementCb(typedReadout(this.typed))
    else this.onMeasurementCb(rect === null ? '' : formatLength(rect.width))
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this.finished) return
    if (this.stage.kind === 'idle') {
      const at = this.idleFrame(snap, ray)
      if (at === null) return
      this.stage = { kind: 'anchored', frame: at.frame, corner: at.point }
      this.lastCursor = at.point
      this.typed = ''
      this.hideGhost()
      this.onMeasurementCb('')
      return
    }
    const { frame, corner } = this.stage
    const cursor = this.anchoredCursor(snap, ray, frame)
    if (cursor === null) return
    // A pending typed width is what the ghost shows, so it is what the
    // click places (on the side the click is on), or refuses when it is
    // under the minimum.
    const typed = this.typedWidth()
    if (typed !== null && this.refuseBadSize(typed)) return
    const rect = pictureRect(frame, corner, cursor, this.aspect, typed)
    // A second click on the corner itself has no size: stay anchored.
    if (rect === null) return
    this.commit(frame, rect.origin, rect.width)
  }

  onKey(ev: KeyboardEvent): void {
    if (this.finished) return
    if (ev.key === 'Escape') {
      if (this.stage.kind === 'anchored') {
        this.stage = { kind: 'idle' }
        this.typed = ''
        this.lastCursor = null
        this.hideGhost()
        this.onMeasurementCb('')
        return
      }
      if (this.idlePlaneLock !== null) {
        this.idlePlaneLock = null
        return
      }
      this.escPressed = true
      this.cancel()
      return
    }
    if (this.stage.kind === 'idle') {
      if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
        if (ev.repeat) return
        this.idlePlaneLock = nextIdlePlaneLock(this.idlePlaneLock, ev.key)
      }
      return
    }
    if (ev.key === 'Enter') {
      const typed = this.typedWidth()
      if (typed === null || this.refuseBadSize(typed)) return
      const { frame, corner } = this.stage
      const rect = pictureRect(frame, corner, this.lastCursor ?? corner, this.aspect, typed)
      if (rect === null) return
      this.commit(frame, rect.origin, rect.width)
      return
    }
    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      const { frame, corner } = this.stage
      this.redrawAnchored(frame, corner, this.lastCursor ?? corner)
    }
  }

  private commit(frame: PictureFrame, origin: V3, width: number): void {
    // Too small for the kernel to build the face: say so and stay armed.
    if (this.refuseBadSize(width)) return
    let raw: unknown
    try {
      raw = this.wasmScene.place_image(
        this.placement.bytes,
        this.placement.format,
        this.placement.name,
        new Float64Array(origin),
        new Float64Array(frame.u),
        new Float64Array(frame.v),
        width,
      )
    } catch (err) {
      // A refusal leaves the document untouched; say so and stay armed.
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      return
    }
    const result = parsePlacedImage(raw)
    this.teardown()
    this.finished = true
    this.onMeasurementCb('')
    this.onPlaced(result)
    this.onDone(true, true)
  }

  /** Also invoked by the ToolController when another tool takes over. */
  cancel(): void {
    this.teardown()
    if (this.finished) return
    this.finished = true
    this.onMeasurementCb('')
    this.onDone(false, this.escPressed)
  }

  onDocumentReset(): void {
    this.stage = { kind: 'idle' }
    this.typed = ''
    this.lastCursor = null
    this.hideGhost()
    // A new document's handles can reuse the old one's: no memoized pick
    // or hover memory may outlive it.
    this.pickCache.clear()
  }

  /** Test seam: the current stage's kind and lock. */
  debugState(): { stage: Stage['kind']; lock: 0 | 1 | 2 | null; typed: string } {
    return { stage: this.stage.kind, lock: this.idlePlaneLock, typed: this.typed }
  }

  /** The ghost's world corners (lower-left, lower-right, upper-right,
   *  upper-left), or null when hidden. Test seam. */
  ghostCorners(): [V3, V3, V3, V3] | null {
    if (this.ghostQuad === null || !this.ghostQuad.visible) return null
    const m = this.ghostQuad.matrixWorld
    const at = (x: number, y: number): V3 => {
      const p = new THREE.Vector3(x, y, 0).applyMatrix4(m)
      return [p.x, p.y, p.z]
    }
    return [at(0, 0), at(1, 0), at(1, 1), at(0, 1)]
  }

  private ensureGhost(): THREE.Group {
    if (this.ghost !== null) return this.ghost
    const root = new THREE.Group()
    // A unit quad over [0,1]², scaled and posed per frame by its matrix.
    const geom = new THREE.BufferGeometry()
    geom.setAttribute('position', new THREE.Float32BufferAttribute([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0], 3))
    geom.setAttribute('uv', new THREE.Float32BufferAttribute([0, 0, 1, 0, 1, 1, 0, 1], 2))
    geom.setIndex([0, 1, 2, 0, 2, 3])
    const mat = new THREE.MeshBasicMaterial({
      ...(this.placement.texture !== null ? { map: this.placement.texture } : { color: 0xffffff }),
      transparent: true,
      opacity: 0.75,
      side: THREE.DoubleSide,
      depthWrite: false,
      polygonOffset: true,
      polygonOffsetFactor: -2,
      polygonOffsetUnits: -2,
    })
    const quad = new THREE.Mesh(geom, mat)
    quad.matrixAutoUpdate = false
    const outlineGeom = new THREE.BufferGeometry()
    outlineGeom.setAttribute('position', new THREE.Float32BufferAttribute(
      [0, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0], 3,
    ))
    const outline = new THREE.LineSegments(outlineGeom, new THREE.LineBasicMaterial({ color: 0x2f6fde, depthTest: false }))
    outline.renderOrder = 3
    quad.add(outline)
    root.add(quad)
    this.preview.add(root)
    this.ghost = root
    this.ghostQuad = quad
    return root
  }

  private drawGhost(frame: PictureFrame, origin: V3, width: number, height: number): void {
    this.ensureGhost()
    const quad = this.ghostQuad as THREE.Mesh
    const [ux, uy, uz] = scale(frame.u, width)
    const [vx, vy, vz] = scale(frame.v, height)
    const [nx, ny, nz] = frame.normal
    quad.matrix.set(
      ux, vx, nx, origin[0],
      uy, vy, ny, origin[1],
      uz, vz, nz, origin[2],
      0, 0, 0, 1,
    )
    quad.matrixWorldNeedsUpdate = true
    quad.updateMatrixWorld(true)
    quad.visible = true
  }

  private hideGhost(): void {
    if (this.ghostQuad !== null) this.ghostQuad.visible = false
  }

  private teardown(): void {
    if (this.ghost !== null) {
      clearPreview(this.preview)
      this.ghost = null
      this.ghostQuad = null
    }
    this.placement.texture?.dispose()
  }
}

/** Parse `Scene.place_image`'s plain-JS result. Exported for tests. */
export function parsePlacedImage(raw: unknown): PlacedImage {
  const obj = (raw ?? {}) as Record<string, unknown>
  const big = (v: unknown): bigint => (typeof v === 'bigint' ? v : BigInt(typeof v === 'number' || typeof v === 'string' ? v : 0))
  const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0)
  return {
    definition: big(obj.definition),
    instance: big(obj.instance),
    width: num(obj.width),
    height: num(obj.height),
  }
}
