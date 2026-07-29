/**
 * PositionTextureTool — SketchUp's fixed-pin Position Texture, over
 * `kernel::UvFrame` (paint-tool design §3, reworked to the tile-corner pin
 * model after the third playtest).
 *
 * ## The tile model
 *
 * All three pins are CORNERS of one texture tile laid on the face:
 *
 *   - RED sits on the UV lattice point nearest the entry click — always on
 *     (or within half a tile of) the clicked face, never at some remote
 *     projection of the world origin.
 *   - GREEN is the adjacent tile corner one tile along U (`(m+1, n)`).
 *   - BLUE is the adjacent tile corner one tile along V (`(m, n+1)`).
 *
 * The tile itself is outlined (red → green → far corner → blue), and a
 * DASHED rectangle shows where the material's natural 1× tile would sit at
 * the current angle — so "where is 1× scale" is literally visible: drag
 * green until the solid outline meets the dashed one, or type `1x`.
 *
 * ## Absolute rotation and scale
 *
 * Rotation and scale are ABSOLUTE (see `frameAbsolute` in uvFrameMath.ts):
 * the angle is measured against the face's own planar axes — the session's
 * local basis IS `tessellatePlaneBasis(normal)`, the axes an untextured
 * face's default projection aligns to, so 0° means "as the default lays
 * it" and typing `45` puts the texture at 45° regardless of where any drag
 * started. Scale 1 means the tile renders at the material's natural world
 * size. The measurement readout shows the current absolute `×scale angle°`
 * whenever the session is open.
 *
 * ## Gesture
 *
 *   1. Click a textured face (its own material, else the object's base,
 *      must carry an image texture) — enters positioning mode AND starts an
 *      immediate translate drag from the click point. Clicking an
 *      untextured/unpainted face toasts and stays idle.
 *   2. While positioning:
 *        - Drag the RED pin: the pin FOLLOWS the cursor (it lands exactly
 *          where you point, including on inference snaps — endpoints,
 *          midpoints, centers — which the snap service constrains to the
 *          face's plane via `snapConstraint`). Dragging anywhere else on
 *          the face pans the decal 1:1 under the cursor, snaps honored the
 *          same way.
 *        - Drag GREEN: the tile corner follows the cursor; the decal
 *          scales+rotates rigidly about the fixed red pin, preserving its
 *          shear/aspect.
 *        - Drag BLUE: the corner follows the cursor; only the V edge moves
 *          (shear + V-scale about the fixed red pin); red and green stay
 *          put.
 *      Every drag is LIVE PREVIEW ONLY (`SceneRenderer.previewFaceUv`
 *      patches the rendered `uv` buffer in place — no kernel call).
 *   3. Typed entry works WHENEVER the session is open — no drag needed
 *      (the third playtest's headline ask). A bare number is absolute
 *      DEGREES; a number with a trailing `x` (`2x`, `0.5x`) is an absolute
 *      SCALE factor; Tab toggles what a bare number means. With no grab
 *      held the value applies to the green (rotate/scale) quantities — or
 *      to blue's (skew degrees / V-scale) after a plain click on the blue
 *      pin. During a red (translate) grab a typed value is a LENGTH along
 *      the current drag direction. Enter applies the typed value exactly;
 *      a further Enter (empty buffer) commits the session.
 *   4. Enter (empty buffer), or a click away from the face, commits the
 *      CURRENT preview frame as ONE kernel call (`set_face_uv_frame`) —
 *      one undo step for the whole session. An untouched session commits
 *      nothing.
 *   5. Esc clears an in-progress typed buffer first; otherwise reverts the
 *      live preview to the exact pre-session frame and exits.
 *
 * ## The threshold gate (load-bearing — do not remove)
 *
 * The Viewport's captured-key branch re-calls `onPointerMove` with the
 * CACHED press-time ray after every keystroke ("Live re-lock"). During a
 * click-hold-and-type gesture the pointer never actually moves, so those
 * replays hand this tool the exact press ray again and again. Nothing is
 * written (preview, drag state, readout) until the RAY's own plane hit has
 * traveled past the standard click/drag threshold from `pressLocal`;
 * `hasMoved` latches once. The travel test deliberately uses the RAW ray
 * hit, never a snapped point — a snap can yank the mapped point past the
 * threshold while the mouse hasn't moved at all, which would let the
 * replay pollute a typed commit.
 *
 * ## Component instances
 *
 * `set_face_uv_frame` already accepts a definition member's object/face.
 * The pointer RAY is mapped into definition-local space via the entered
 * instance's pose INVERSE (`_rayToSessionSpace`) — every math path then
 * runs unchanged in the space it already assumes, exact under non-uniform
 * and mirrored poses alike (a UV frame maps by full affine conjugation;
 * such instances are positioned, never refused). Positioning an instanced
 * face requires being INSIDE the component (`setEditContext`/
 * `setFaceEligibility`, the same channel every other face tool uses):
 * `set_face_uv_frame` on a member changes every placement of the
 * definition, and SketchUp likewise makes you open the component first.
 *
 * SketchUp's fourth (yellow, perspective) pin is deliberately not
 * implemented: `kernel::UvFrame` is purely affine (no `w` row/divide), so
 * a projective warp needs a kernel struct change and a file-format bump —
 * the maintainer's call, not this tool's.
 */

import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import { screenConstantWorldHalfFromWorldPerPixel } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { rayPlaneIntersect, invertAffine3x4, applyAffine3x4, applyAffine3x4Linear, type V3 } from '../viewport/geoHelpers'
import { DRAG_MOVE_THRESHOLD_PX } from '../viewport/dragMove'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import type { FaceEligible } from './faceDraw'
import {
  frameToLocal2D,
  local2DToFrame,
  local2ToWorld,
  worldToLocal2,
  uvPointLocal,
  translateLocal,
  scaleRotateLocal,
  shearScaleLocal,
  frameAbsolute,
  tessellatePlaneBasis,
  planarDefaultFrame,
  frameToArray,
  arrayToFrame,
  matVec,
  rotation2,
  type Local2,
  type UvFrameComponents,
} from './uvFrameMath'
import { editScaleOrRotateBuffer, editLengthBuffer, isLengthInputKey, parseScaleOrRotate } from './moveInput'
import { parseLengthToMeters, getLengthUnit, formatLength, typedReadout } from '../settings/units'

/** `u64::MAX` as a BigInt — the "default / unpainted" sentinel (same
 *  convention `MaterialPalette`/`PaintTool` use). */
const MATERIAL_SENTINEL: bigint = BigInt('18446744073709551615')

/**
 * Target on-screen half-extent of the pin gizmo boxes, in CSS pixels — held
 * CONSTANT regardless of camera distance, exactly `ScaleTool`'s
 * `GRIP_SCREEN_PX` posture (see `updateGripScale`).
 */
const GIZMO_SCREEN_PX = 8
/** Floor on the gizmo's rendered WORLD half-extent, in meters — guards a
 *  literal zero/negative scale at a degenerate viewport (mirrors ScaleTool's
 *  `MIN_GRIP_WORLD_HALF`). */
const MIN_GIZMO_WORLD_HALF = 1e-5
/** Placeholder half-extent (meters) before the render loop's first
 *  `updateGripScale` tick, and the pick-tolerance fallback for the same
 *  window (unit tests never drive a render loop — mirrors ScaleTool's
 *  `FALLBACK_GRIP_HALF_M`). */
const FALLBACK_GIZMO_HALF_M = 0.02
/** Pick tolerance around a pin, as a multiple of its rendered on-screen
 *  half-extent (mirrors ScaleTool's `GRIP_PICK_MULTIPLIER`). */
const GIZMO_PICK_MULTIPLIER = 3
/** Fallback world-space click/drag threshold (meters) before the first
 *  render tick has cached the camera/viewport — see `_worldDragThresholdAt`. */
const FALLBACK_DRAG_THRESHOLD_M = 0.01

/** Snap kinds that name a discrete POINT a dragged pin should land on
 *  exactly (the endpoint-snap "pretends to snap but doesn't take" defect).
 *  Broad-area kinds (`on-face`, `ground`) are excluded — they carry no
 *  point the ray∩plane hit doesn't already give. */
const POINT_SNAP_KINDS = new Set([
  'endpoint',
  'midpoint',
  'center',
  'quadrant',
  'intersection',
  'tangent',
  'on-edge',
  'on-guide',
  'on-axis',
])

export type OnPositionCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Called on every live preview frame. Implemented by Viewport as a thin
 *  wrapper over `SceneRenderer.previewFaceUv`. */
export type PreviewFaceUv = (object: bigint, face: bigint, frame: UvFrameComponents) => void

type Grab =
  | {
      kind: 'translate'
      /** The local point the drag is anchored to: the RED PIN's own position
       *  for a pin grab (the pin follows the cursor — a snapped drag lands
       *  the pin EXACTLY on the snap), the press-ray hit for a face-body
       *  drag (1:1 pan). */
      startLocal: readonly [number, number]
      baseLocal: Local2
      /** `session.dirty` as it stood right before this grab began — a
       *  below-threshold release restores it, so a real earlier drag in the
       *  SAME session stays committed even if this grab turns out to be a
       *  click. */
      dirtyBeforeGrab: boolean
      /** The press ray's own plane-hit at grab start — the reference the
       *  threshold gate measures RAW ray travel against (see the class doc
       *  comment; never a snapped point). */
      pressLocal: readonly [number, number]
      /** Latches true the first time the RAW ray hit travels past the
       *  click/drag threshold from `pressLocal`. Nothing is written before
       *  that. Flips exactly once per grab. */
      hasMoved: boolean
      /** The most recent live delta applied (local units) — what a typed
       *  length scales along. `[0, 0]` before any real move. */
      lastDelta: readonly [number, number]
    }
  | {
      kind: 'scaleRotate' | 'shear'
      /** The red pin's local position — the fixed pivot. */
      pivot: readonly [number, number]
      /** The grabbed tile corner's local position at grab start (green:
       *  `(m+1, n)`; blue: `(m, n+1)`). */
      oldHandle: readonly [number, number]
      baseLocal: Local2
      dirtyBeforeGrab: boolean
      /** See the translate variant — same threshold-gate reference. */
      pressLocal: readonly [number, number]
      hasMoved: boolean
    }

interface Session {
  object: bigint
  face: bigint
  anchor: V3
  normal: V3
  /** The session's in-plane basis — ALWAYS `tessellatePlaneBasis(normal)`,
   *  the axes an untextured face's default projection aligns to, so the
   *  local `+a` axis IS the absolute-rotation reference the user sees
   *  (angle 0 = the default lay). In DEFINITION-local space for an
   *  instance session, like everything else here. */
  uAx: V3
  vAx: V3
  /** The pre-gesture frame, for an exact Esc revert. */
  originalLocal: Local2
  /** The live frame (updated continuously during a drag). */
  workingLocal: Local2
  grab: Grab | null
  /** Whether anything actually changed — an untouched session commits
   *  nothing (no spurious undo entry). */
  dirty: boolean
  /** The RED pin's UV lattice point `(m, n)` — the lattice point nearest
   *  the entry click's own hit, so the pins spawn ON the clicked face (or
   *  within half a tile of the click for a tile larger than the face),
   *  never at the world origin. Green is `(m+1, n)`, blue `(m, n+1)`.
   *  Fixed for the session: gestures preserve the pivot's UV, so the pin
   *  stays glued to this lattice point of the moving pattern. */
  pinUv: readonly [number, number]
  /** The effective material's natural tile world size — the absolute-scale
   *  reference (`scale 1` = tile edge equals this). */
  worldW: number
  worldH: number
  /** Which pin a typed value targets when NO grab is held: green
   *  (absolute rotation / uniform scale — the default) or blue (absolute
   *  skew / V-scale). A plain click on either pin selects it. */
  typedTarget: 'green' | 'blue'
  /** `history_generation()` at session-open — the identity guard against a
   *  mid-session Cmd+Z/Redo, which bypasses tool routing and can leave the
   *  cached basis/handles stale (see `_abortIfHistoryMoved`). */
  historyGen: string
  /** The component instance this session positions THROUGH, or `null` for
   *  a plain world object. When set, everything local here is in the
   *  instance's DEFINITION space. */
  instance: bigint | null
  /** The instance's pose (row-major 3x4), `null` iff `instance` is. */
  instancePose: number[] | null
  /** Its inverse — maps WORLD pointer rays INTO definition space. */
  instancePoseInv: number[] | null
}

function dist3(a: V3, b: V3): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])
}

/** Closest distance from an (infinite, forward-only) ray to a world point —
 *  the pin hit test. `rayDir` need not be unit length. */
function rayPointDistance(ray: Ray, point: V3): number {
  const [ox, oy, oz] = ray.origin
  const [dx, dy, dz] = ray.direction
  const wx = point[0] - ox
  const wy = point[1] - oy
  const wz = point[2] - oz
  const dd = dx * dx + dy * dy + dz * dz
  const t = dd > 1e-12 ? Math.max(0, (wx * dx + wy * dy + wz * dz) / dd) : 0
  const cx = ox + dx * t
  const cy = oy + dy * t
  const cz = oz + dz * t
  return Math.hypot(point[0] - cx, point[1] - cy, point[2] - cz)
}

const DEG = 180 / Math.PI

export class PositionTextureTool implements Tool {
  readonly name = 'Position Texture'

  private wasmScene: WasmScene
  private gizmoGroup: THREE.Group
  private previewFaceUv: PreviewFaceUv
  private onCommit: OnPositionCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  private session: Session | null = null

  private pinMesh: THREE.Mesh | null = null
  private handleMesh: THREE.Mesh | null = null
  private blueHandleMesh: THREE.Mesh | null = null
  /** The tile's own outline (red → green → far corner → blue). */
  private tileOutline: THREE.LineLoop | null = null
  /** The dashed natural-size (1×) reference rectangle — the visible answer
   *  to "where is 1× scale": anchored at the red pin, oriented to the
   *  tile's CURRENT angle, sized to the material's natural tile. When the
   *  texture is at 1× and unsheared the solid tile outline coincides with
   *  it. */
  private refOutline: THREE.LineLoop | null = null

  /** The `worldPerPixel` callback cached by `updateGripScale` (once per
   *  render tick, the same wiring `ScaleTool.updateGripScale` uses) so
   *  pick-tolerance/threshold math matches what's on screen. `null` until
   *  the first tick (unit tests) — readers fall back to fixed placeholders. */
  private _pickWorldPerPixel: ((dist: number) => number) | null = null

  /** VCB buffer — raw string, meaningful whenever a session is open (see
   *  `capturingInput`). */
  private typed: string = ''
  /** What a BARE typed number means — degrees (default) or a scale factor;
   *  Tab toggles. A trailing `x` always forces scale. */
  private typedMode: 'rotate' | 'scale' = 'rotate'

  /** The current editing context — pushed by the Viewport via
   *  `setEditContext`, mirroring PushPullTool. */
  private _editContext: EditContext = { kind: 'top' }
  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
    this.cancel()
  }

  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
  }

  /** Optional richer eligibility, injected by the Viewport — same channel
   *  PushPullTool/the draw tools use. Null = the fallback policy below. */
  private _faceEligible: FaceEligible | null = null
  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }

  /**
   * May this pick be positioned? With an injected predicate (the live app),
   * defers to it entirely. Without one (unit tests): plain objects are
   * unconditionally eligible; an instanced pick only when it's the
   * currently entered instance.
   */
  private _isEligible(object: bigint, instance: bigint | undefined): boolean {
    if (this._faceEligible !== null) return this._faceEligible(object, instance)
    if (instance !== undefined) return instance === this._activeInstance
    return true
  }

  private _ineligibleFaceHint(instance: bigint | undefined): string {
    if (instance !== undefined) return 'Enter the component to position its texture.'
    return 'Position Texture is scoped to what you are editing — press Esc to step out first'
  }

  constructor(
    wasmScene: WasmScene,
    gizmoGroup: THREE.Group,
    previewFaceUv: PreviewFaceUv,
    onCommit: OnPositionCommit,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.gizmoGroup = gizmoGroup
    this.previewFaceUv = previewFaceUv
    this.onCommit = onCommit
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  statusHint(): string {
    const s = this.session
    if (s === null) {
      return 'Click a textured face to position its texture.'
    }
    if (s.grab !== null && s.grab.kind === 'scaleRotate') {
      return this.typedMode === 'rotate'
        ? 'Drag the corner, or type the absolute angle in degrees (Tab for scale) — Enter sets it, Esc clears.'
        : 'Drag the corner, or type an absolute scale factor (Tab for degrees) — Enter sets it, Esc clears.'
    }
    if (s.grab !== null && s.grab.kind === 'shear') {
      return this.typedMode === 'rotate'
        ? 'Drag the corner to shear, or type the absolute skew in degrees (Tab for V-scale) — Enter sets it, Esc clears.'
        : 'Drag the corner to shear, or type an absolute V-scale factor (Tab for degrees) — Enter sets it, Esc clears.'
    }
    if (s.grab !== null && s.grab.kind === 'translate') {
      return 'Drag to move (snaps apply), or type an exact length — Enter sets it, Esc clears.'
    }
    const target = s.typedTarget === 'blue' ? 'skew/V-scale (blue)' : 'angle/scale (green)'
    return `Drag a corner pin, or type the absolute ${target}: degrees, or 2x for scale. Enter commits, Esc cancels.`
  }

  /**
   * Numeric VCB capture: true whenever a SESSION is open — typed absolute
   * rotation/scale must work without any drag (the third playtest's
   * headline requirement), so number keys stop switching tools the moment
   * the pins are up. `capturesKey` narrows this to the keys the buffer
   * actually uses.
   */
  capturingInput(): boolean {
    return this.session !== null
  }

  /** Per-key refinement of `capturingInput` (see `Tool.capturesKey`): only
   *  the keys the current entry form actually accepts are captured. */
  capturesKey(key: string): boolean {
    const s = this.session
    if (s === null) return false
    if (key === 'Enter' || key === 'Tab' || key === 'Backspace') return true
    if (s.grab !== null && s.grab.kind === 'translate') return isLengthInputKey(key)
    return /^[0-9]$/.test(key) || key === '.' || key === '-' || key === 'x' || key === 'X'
  }

  /**
   * See the class doc comment's "threshold gate" section: nothing is
   * written until the RAW ray hit travels past the click/drag threshold
   * from `pressLocal`. Once `hasMoved` latches, the applied point is the
   * SNAPPED point when a point snap is live (`_snapLocal`) — that is what
   * makes an endpoint snap actually TAKE — and the raw hit otherwise.
   */
  onPointerMove(snap: Snap | null, ray: Ray): void {
    const s = this.session
    if (s === null || s.grab === null) return
    if (this._abortIfHistoryMoved()) return

    const localRay = this._rayToSessionSpace(ray)
    const hit = rayPlaneIntersect(localRay.origin, localRay.direction, s.anchor, s.normal)
    if (hit === null) return
    const rawCur = worldToLocal2(s.anchor, s.uAx, s.vAx, hit)

    if (!s.grab.hasMoved) {
      const pressWorld = this._localToWorld(local2ToWorld(s.anchor, s.uAx, s.vAx, s.grab.pressLocal))
      const soFar: [number, number] = [rawCur[0] - s.grab.pressLocal[0], rawCur[1] - s.grab.pressLocal[1]]
      const traveled = this._worldLen(soFar)
      const threshold = this._worldDragThresholdAt(dist3(ray.origin, pressWorld))
      if (traveled < threshold) return
      s.grab.hasMoved = true
    }

    const cur = this._snapLocal(snap) ?? rawCur

    if (s.grab.kind === 'translate') {
      const delta: [number, number] = [cur[0] - s.grab.startLocal[0], cur[1] - s.grab.startLocal[1]]
      s.grab.lastDelta = delta
      this._applyPreview(translateLocal(s.grab.baseLocal, delta))
      this._reportTypedOrLiveTranslate(s.grab)
    } else {
      const result = s.grab.kind === 'scaleRotate'
        ? scaleRotateLocal(s.grab.baseLocal, s.grab.pivot, s.grab.oldHandle, cur)
        : shearScaleLocal(s.grab.baseLocal, s.grab.pivot, cur)
      if (result === null) return
      this._applyPreview(result)
      this._reportTypedOrLiveAbsolute(s.grab.kind)
    }
  }

  onPointerDown(_snap: Snap | null, ray: Ray): void {
    if (this.session === null) {
      this._tryEnter(ray)
      return
    }
    this._onPointerDownInSession(ray)
  }

  onPointerUp(_snap: Snap | null, _ray: Ray): void {
    const s = this.session
    if (s === null || s.grab === null) return
    // A mid-drag external Cmd+Z/Redo can land between the last move and
    // this release — same write-path guard every other continue entry point
    // takes (cancel paths deliberately do NOT; see their doc comments).
    if (this._abortIfHistoryMoved()) return
    this._resolveGrabRelease(s.grab)
    s.grab = null
    this._resetTyped()
    this._reportIdleAbsolute()
  }

  onKey(ev: KeyboardEvent): void {
    const s = this.session
    if (s === null) return

    if (ev.key === 'Enter') {
      // A non-empty typed buffer sets an EXACT absolute value (or an exact
      // length during a translate grab) and leaves the session open; a
      // buffer-empty Enter commits the whole session.
      if (this.typed !== '') {
        if (s.grab !== null && s.grab.kind === 'translate') {
          this._commitTypedTranslate(s.grab)
        } else {
          const kind = s.grab !== null
            ? s.grab.kind
            : s.typedTarget === 'blue' ? 'shear' : 'scaleRotate'
          this._applyTypedAbsolute(kind)
        }
        return
      }
      this._commitAndExit()
      return
    }

    if (ev.key === 'Escape') {
      // A non-empty typed buffer clears FIRST — only a SECOND Escape falls
      // through to the session cancel (the codebase's two-stage Escape).
      if (this.typed !== '') {
        this._resetTyped()
        if (s.grab !== null && s.grab.kind === 'translate') {
          this._reportLiveTranslate(s.grab)
        } else {
          this._reportIdleAbsolute()
        }
        return
      }
      // Deliberately NOT routed through `_abortIfHistoryMoved` — a cancel
      // never writes; alarming the user over an unrelated undo elsewhere
      // in the document would be wrong. `_cancelSession` carries its own
      // recycled-handle guard.
      this._cancelSession()
      return
    }

    if (ev.key === 'Tab') {
      if (s.grab !== null && s.grab.kind === 'translate') return
      ev.preventDefault()
      this.typedMode = this.typedMode === 'rotate' ? 'scale' : 'rotate'
      this._reportTypedOrLiveAbsolute(this._typedKind())
      return
    }

    if (s.grab !== null && s.grab.kind === 'translate' && isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this._reportTypedOrLiveTranslate(s.grab)
      return
    }

    if (
      (s.grab === null || s.grab.kind === 'scaleRotate' || s.grab.kind === 'shear') &&
      ((ev.key >= '0' && ev.key <= '9') || ev.key === '.' || ev.key === '-' ||
        ev.key === 'x' || ev.key === 'X' || ev.key === 'Backspace')
    ) {
      this.typed = editScaleOrRotateBuffer(this.typed, ev.key)
      this._reportTypedOrLiveAbsolute(this._typedKind())
    }
  }

  cancel(): void {
    // Same quiet posture as Esc: a tool switch must not linger an
    // uncommitted preview, but it's only a revert — no write to guard.
    this._cancelSession()
  }

  /**
   * Constrains the snap service to the session's face plane while
   * positioning (same mechanism RectangleTool's anchored face mode uses) —
   * so the endpoint/midpoint snaps a dragged pin lands on are ON the plane
   * the drag math projects to, not on some occluded off-plane geometry.
   */
  snapConstraint(): { constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null {
    const s = this.session
    if (s === null) return null
    const point = this._localToWorld(s.anchor)
    return { constraintPlane: { point: [point[0], point[1], point[2]], normal: this._worldNormal() } }
  }

  // ───────────────────────────────────────────────────────── internals

  /** Which pin kind a typed value currently targets (for the readout). */
  private _typedKind(): 'scaleRotate' | 'shear' {
    const s = this.session
    if (s !== null && s.grab !== null && s.grab.kind === 'shear') return 'shear'
    if (s !== null && s.grab === null && s.typedTarget === 'blue') return 'shear'
    return 'scaleRotate'
  }

  private _tryEnter(ray: Ray): void {
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (pick === undefined) return
    try {
      const object = pick.object()
      const face = pick.face()
      const instance = pick.instance()

      if (!this._isEligible(object, instance)) {
        this.onToast(this._ineligibleFaceHint(instance))
        return
      }

      let instancePose: number[] | null = null
      let instancePoseInv: number[] | null = null
      if (instance !== undefined) {
        const poseArr = this.wasmScene.instance_pose(instance)
        if (poseArr === undefined) return // stale instance — silent miss
        const inv = invertAffine3x4(poseArr)
        if (inv === null) return // degenerate pose — unreachable for a live instance
        instancePose = Array.from(poseArr)
        instancePoseInv = inv
      }

      const texture = this._effectiveTexture(object, face)
      if (texture === null) {
        this.onToast('Select a textured face to position its texture')
        return
      }
      const planeArr = this.wasmScene.face_plane(object, face)
      if (planeArr === undefined) return
      const anchor: V3 = [planeArr[0], planeArr[1], planeArr[2]]
      const normal: V3 = [planeArr[3], planeArr[4], planeArr[5]]
      // The session basis IS the absolute-rotation reference — see the
      // Session doc comment.
      const basis = tessellatePlaneBasis(normal)

      const raw = this.wasmScene.face_uv_frame(object, face)
      if (raw === undefined) return
      const frame: UvFrameComponents =
        raw.length === 8 ? arrayToFrame(raw) : planarDefaultFrame(normal, texture.worldW, texture.worldH)
      const local = frameToLocal2D(frame, anchor, basis.u, basis.v)

      // RED pin placement: the lower-left corner of the texture tile that
      // CONTAINS the entry click's own plane hit — never the world origin.
      //
      // `Math.floor`, not `Math.round`: the pin names a tile, not just a
      // point. `_cornersLocal` draws the tile spanning [m, m+1] x [n, n+1],
      // so rounding to the NEAREST lattice point put the whole gizmo one
      // full tile off the clicked spot whenever the hit fell past a tile's
      // midpoint — with a single-repeat texture on a face, that is the
      // adjacent square floating clean off the face, sharing only its far
      // edge, and every rotation then pivoted about that off-face corner.
      // Flooring is also the correct choice for negative UVs, where
      // rounding and flooring disagree about which tile a point is in.
      const localRay = instancePoseInv !== null
        ? { origin: applyAffine3x4(instancePoseInv, ray.origin), direction: applyAffine3x4Linear(instancePoseInv, ray.direction) }
        : ray
      const hit = rayPlaneIntersect(localRay.origin, localRay.direction, anchor, normal)
      const hitLocal = hit !== null ? worldToLocal2(anchor, basis.u, basis.v, hit) : [0, 0] as [number, number]
      const uvAtHit = matVec(local.m, hitLocal)
      const pinUv: [number, number] = [
        Math.floor(uvAtHit[0] + local.off[0]),
        Math.floor(uvAtHit[1] + local.off[1]),
      ]

      this.session = {
        object,
        face,
        anchor,
        normal,
        uAx: basis.u,
        vAx: basis.v,
        originalLocal: local,
        workingLocal: local,
        grab: null,
        dirty: false,
        pinUv,
        worldW: texture.worldW > 0 ? texture.worldW : 1,
        worldH: texture.worldH > 0 ? texture.worldH : 1,
        typedTarget: 'green',
        historyGen: this.wasmScene.history_generation().toString(),
        instance: instance ?? null,
        instancePose,
        instancePoseInv,
      }
      this._buildGizmo()
      this._updateGizmo(local)
      this._beginTranslateDrag(ray, null)
      this._reportIdleAbsolute()
    } finally {
      pick.free()
    }
  }

  /** The effective material's texture world-size, or `null` if the face's
   *  effective material (own, else object base) is unpainted or untextured. */
  private _effectiveTexture(object: bigint, face: bigint): { worldW: number; worldH: number } | null {
    const info = this.wasmScene.face_material(object, face)
    if (info === undefined) return null
    let effective: bigint
    try {
      const own = info.face()
      effective = own !== MATERIAL_SENTINEL ? own : info.object_default()
    } finally {
      info.free()
    }
    if (effective === MATERIAL_SENTINEL) return null
    const mat = this.wasmScene.material_info(effective)
    if (mat === undefined) return null
    try {
      if (!mat.has_texture()) return null
      return { worldW: mat.world_w(), worldH: mat.world_h() }
    } finally {
      mat.free()
    }
  }

  private _onPointerDownInSession(ray: Ray): void {
    const s = this.session
    if (s === null || s.grab !== null) return
    if (this._abortIfHistoryMoved()) return

    const corners = this._cornersLocal(s.workingLocal)
    if (corners !== null) {
      const { pin, green, blue } = corners
      const pinWorld = this._localToWorld(local2ToWorld(s.anchor, s.uAx, s.vAx, pin))
      const greenWorld = this._localToWorld(local2ToWorld(s.anchor, s.uAx, s.vAx, green))
      const blueWorld = this._localToWorld(local2ToWorld(s.anchor, s.uAx, s.vAx, blue))

      if (rayPointDistance(ray, greenWorld) <= this._pickToleranceAt(ray, greenWorld)) {
        s.typedTarget = 'green'
        this._grabHandle('scaleRotate', pin, green, ray)
        return
      }
      if (rayPointDistance(ray, blueWorld) <= this._pickToleranceAt(ray, blueWorld)) {
        s.typedTarget = 'blue'
        this._grabHandle('shear', pin, blue, ray)
        return
      }
      if (rayPointDistance(ray, pinWorld) <= this._pickToleranceAt(ray, pinWorld)) {
        // Grab the RED PIN itself: the pin follows the cursor (lands
        // exactly on a snapped point), rather than panning 1:1 from the
        // press offset.
        this._beginTranslateDrag(ray, pin)
        return
      }
    }

    // Still on the same face — AND the same instance placement for an
    // instance session (`object`/`face` alone are shared by every placement
    // of the same definition member). Otherwise: click-away commit.
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (pick !== undefined) {
      try {
        const sameInstance = (pick.instance() ?? null) === s.instance
        if (pick.object() === s.object && pick.face() === s.face && sameInstance) {
          this._beginTranslateDrag(ray, null)
          return
        }
      } finally {
        pick.free()
      }
    }
    this._commitAndExit()
  }

  /** Shared grab-open for the green/blue corner pins. */
  private _grabHandle(kind: 'scaleRotate' | 'shear', pivot: [number, number], oldHandle: [number, number], ray: Ray): void {
    const s = this.session
    if (s === null) return
    this._resetTyped()
    const localRay = this._rayToSessionSpace(ray)
    // The press ray's OWN plane hit — may sit anywhere within the pick
    // tolerance of the corner, not exactly on it; it is the threshold
    // gate's travel reference (see the class doc comment). Falls back to
    // the corner itself on the (essentially unreachable) re-intersect miss.
    const pressHit = rayPlaneIntersect(localRay.origin, localRay.direction, s.anchor, s.normal)
    const pressLocal = pressHit !== null ? worldToLocal2(s.anchor, s.uAx, s.vAx, pressHit) : oldHandle
    s.grab = {
      kind,
      pivot,
      oldHandle,
      baseLocal: s.workingLocal,
      dirtyBeforeGrab: s.dirty,
      pressLocal,
      hasMoved: false,
    }
    this._reportTypedOrLiveAbsolute(kind)
  }

  /** Opens a translate grab. `pinStart` non-null = the RED PIN was grabbed
   *  (drag anchored to the pin, which then follows the cursor exactly);
   *  null = a face-body drag (anchored to the press hit, 1:1 pan). */
  private _beginTranslateDrag(ray: Ray, pinStart: readonly [number, number] | null): void {
    const s = this.session
    if (s === null) return
    const localRay = this._rayToSessionSpace(ray)
    const hit = rayPlaneIntersect(localRay.origin, localRay.direction, s.anchor, s.normal)
    if (hit === null) return
    const pressLocal = worldToLocal2(s.anchor, s.uAx, s.vAx, hit)
    this._resetTyped()
    this.onMeasurementCb('')
    s.grab = {
      kind: 'translate',
      startLocal: pinStart ?? pressLocal,
      baseLocal: s.workingLocal,
      dirtyBeforeGrab: s.dirty,
      pressLocal,
      hasMoved: false,
      lastDelta: [0, 0],
    }
  }

  /**
   * A grab that ends without ever passing the click/drag threshold
   * (`hasMoved` never latched — see the class doc comment's gate) is a
   * CLICK, not a drag: revert this grab's effect entirely (nothing was
   * previewed anyway) and restore `dirty` to its pre-grab value, so a real
   * earlier drag in the same session stays committed. A grab that DID move
   * keeps whatever the last `onPointerMove` applied.
   */
  private _resolveGrabRelease(grab: Grab): void {
    const s = this.session
    if (s === null) return
    if (grab.hasMoved) return

    s.workingLocal = grab.baseLocal
    s.dirty = grab.dirtyBeforeGrab
    const revert = local2DToFrame(grab.baseLocal, s.anchor, s.uAx, s.vAx)
    this.previewFaceUv(s.object, s.face, revert)
    this._updateGizmo(grab.baseLocal)
  }

  private _applyPreview(local: Local2): void {
    const s = this.session
    if (s === null) return
    s.dirty = true
    const frame = local2DToFrame(local, s.anchor, s.uAx, s.vAx)
    this.previewFaceUv(s.object, s.face, frame)
    this._updateGizmo(local)
    s.workingLocal = local
  }

  private _commitAndExit(): void {
    const s = this.session
    if (s === null) return
    if (this._abortIfHistoryMoved()) return
    if (s.dirty) {
      const frame = local2DToFrame(s.workingLocal, s.anchor, s.uAx, s.vAx)
      try {
        this.wasmScene.set_face_uv_frame(s.object, s.face, new Float64Array(frameToArray(frame)))
        this.onCommit(s.object)
      } catch (err) {
        this._handleError(err)
        // Re-render the object's real committed state so the preview and
        // the (un-committed) document cannot drift.
        this.onCommit(s.object)
      }
    }
    this._resetTyped()
    this.onMeasurementCb('')
    this._clearGizmo()
    this.session = null
  }

  /** Quiet cancel — Escape and `cancel()` route here, with NO generation
   *  check and NO toast (a cancel never writes). It still can't blindly
   *  patch a recycled handle: if `object`/`face` no longer resolve, nothing
   *  of THIS session's preview remains to revert. */
  private _cancelSession(): void {
    const s = this.session
    if (s === null) return
    if (this._faceStillResolves(s.object, s.face)) {
      const revert = local2DToFrame(s.originalLocal, s.anchor, s.uAx, s.vAx)
      this.previewFaceUv(s.object, s.face, revert)
    }
    this._resetTyped()
    this.onMeasurementCb('')
    this._clearGizmo()
    this.session = null
  }

  private _resetTyped(): void {
    this.typed = ''
    this.typedMode = 'rotate'
  }

  private _handleError(err: unknown): void {
    const code = parseKernelErrorCode(err)
    const rawMsg = err instanceof Error ? err.message : String(err)
    const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
    this.onToast(message, code ?? undefined)
  }

  /** Whether `object`/`face` still resolve in the CURRENT document — see
   *  `_abortIfHistoryMoved`. `face_material` is the cheapest probe that
   *  reports an unresolvable handle as `undefined`; wrapped in try/catch
   *  since some wasm getters throw instead. */
  private _faceStillResolves(object: bigint, face: bigint): boolean {
    try {
      const info = this.wasmScene.face_material(object, face)
      if (info === undefined) return false
      info.free()
      return true
    } catch {
      return false
    }
  }

  /**
   * Abort the session cleanly when the document's history generation moved
   * since it opened: no kernel write is safe against a possibly-stale
   * cached basis, and `object`/`face` could have been slotmap-recycled.
   * Reverts the face's in-place preview to `originalLocal` first (unless
   * the handle no longer resolves — then nothing of this session's preview
   * remains, and patching would risk writing into a reused handle), toasts,
   * and closes. Returns whether it aborted. Only called from WRITE/continue
   * entry points — never from Escape/`cancel()` (see `_cancelSession`).
   */
  private _abortIfHistoryMoved(): boolean {
    const s = this.session
    if (s === null || this.wasmScene.history_generation().toString() === s.historyGen) return false
    if (this._faceStillResolves(s.object, s.face)) {
      const revert = local2DToFrame(s.originalLocal, s.anchor, s.uAx, s.vAx)
      this.previewFaceUv(s.object, s.face, revert)
    }
    this.onToast('Position Texture ended — the model changed since positioning started')
    this._resetTyped()
    this.onMeasurementCb('')
    this._clearGizmo()
    this.session = null
    return true
  }

  /** World-space equivalent of the `DRAG_MOVE_THRESHOLD_PX` click/drag
   *  threshold at distance `dist` from the camera (see `updateGripScale`);
   *  fixed fallback before the first render tick. */
  private _worldDragThresholdAt(dist: number): number {
    if (this._pickWorldPerPixel === null) {
      return FALLBACK_DRAG_THRESHOLD_M
    }
    return screenConstantWorldHalfFromWorldPerPixel(
      DRAG_MOVE_THRESHOLD_PX,
      this._pickWorldPerPixel(dist),
    )
  }

  // ──────────────────────────────────────────── in-component ray mapping

  /**
   * Maps a WORLD pointer ray into this session's DEFINITION-local space via
   * the entered instance's pose INVERSE — a no-op for a plain world-object
   * session. The origin maps as a POINT, the direction as a VECTOR. Exact
   * under non-uniform and mirrored poses alike (an affine change of
   * coordinates, not an approximation).
   */
  private _rayToSessionSpace(ray: Ray): Ray {
    const s = this.session
    if (s === null || s.instancePoseInv === null) return ray
    return {
      origin: applyAffine3x4(s.instancePoseInv, ray.origin),
      direction: applyAffine3x4Linear(s.instancePoseInv, ray.direction),
    }
  }

  /** Maps a DEFINITION-local point out to true WORLD space (identity for a
   *  plain session) — every gizmo-rendering and hit-testing site needs it. */
  private _localToWorld(p: V3): V3 {
    const s = this.session
    if (s === null || s.instancePose === null) return p
    return applyAffine3x4(s.instancePose, p)
  }

  /** The session plane's TRUE WORLD normal — the pose's inverse-transpose
   *  applied to the local normal (exact under non-uniform scale), for the
   *  snap service's constraint plane. */
  private _worldNormal(): [number, number, number] {
    const s = this.session
    if (s === null) return [0, 0, 1]
    if (s.instancePoseInv === null) return [s.normal[0], s.normal[1], s.normal[2]]
    const inv = s.instancePoseInv
    const n = s.normal
    const nw: [number, number, number] = [
      inv[0] * n[0] + inv[4] * n[1] + inv[8] * n[2],
      inv[1] * n[0] + inv[5] * n[1] + inv[9] * n[2],
      inv[2] * n[0] + inv[6] * n[1] + inv[10] * n[2],
    ]
    const len = Math.hypot(nw[0], nw[1], nw[2])
    if (len < 1e-12) return [s.normal[0], s.normal[1], s.normal[2]]
    return [nw[0] / len, nw[1] / len, nw[2] / len]
  }

  /** The session-local `(a, b)` position of a live POINT snap, or `null`
   *  when there is none (or it's a broad-area kind). The world snap point
   *  maps through the instance pose inverse, then projects onto the face
   *  plane implicitly (`worldToLocal2` drops the normal component — the
   *  constraint plane keeps it near-zero anyway). */
  private _snapLocal(snap: Snap | null): [number, number] | null {
    const s = this.session
    if (s === null || snap === null || !POINT_SNAP_KINDS.has(snap.kind)) return null
    const world: V3 = [snap.x, snap.y, snap.z]
    const p = s.instancePoseInv !== null ? applyAffine3x4(s.instancePoseInv, world) : world
    return worldToLocal2(s.anchor, s.uAx, s.vAx, p)
  }

  /**
   * TRUE world-space length of the 3D vector for a LOCAL delta `(da, db)` —
   * mapped through the instance pose's linear part when present, so the
   * click/drag threshold and typed lengths mean real meters even under a
   * non-uniformly-scaled instance.
   */
  private _worldLen(deltaLocal: readonly [number, number]): number {
    const s = this.session
    if (s === null) return Math.hypot(deltaLocal[0], deltaLocal[1])
    const localVec: V3 = [
      s.uAx[0] * deltaLocal[0] + s.vAx[0] * deltaLocal[1],
      s.uAx[1] * deltaLocal[0] + s.vAx[1] * deltaLocal[1],
      s.uAx[2] * deltaLocal[0] + s.vAx[2] * deltaLocal[1],
    ]
    if (s.instancePose === null) return Math.hypot(localVec[0], localVec[1], localVec[2])
    const worldVec = applyAffine3x4Linear(s.instancePose, localVec)
    return Math.hypot(worldVec[0], worldVec[1], worldVec[2])
  }

  // ───────────────────────────────────────────────────────────── gizmo

  /** The three tile-corner pins' local positions for `local`, or `null`
   *  when the frame is degenerate (singular — nothing sensible to draw). */
  private _cornersLocal(local: Local2): { pin: [number, number]; green: [number, number]; blue: [number, number] } | null {
    const s = this.session
    if (s === null) return null
    const [m, n] = s.pinUv
    const pin = uvPointLocal(local, [m, n])
    const green = uvPointLocal(local, [m + 1, n])
    const blue = uvPointLocal(local, [m, n + 1])
    if (pin === null || green === null || blue === null) return null
    return { pin, green, blue }
  }

  private _buildGizmo(): void {
    this._clearGizmo()
    const pinMat = new THREE.MeshBasicMaterial({ color: 0xcc2200, depthTest: false })
    const handleMat = new THREE.MeshBasicMaterial({ color: 0x00cc44, depthTest: false })
    const blueHandleMat = new THREE.MeshBasicMaterial({ color: 0x2266ee, depthTest: false })
    this.pinMesh = new THREE.Mesh(new THREE.BoxGeometry(2, 2, 2), pinMat)
    this.pinMesh.renderOrder = 999
    this.pinMesh.scale.setScalar(FALLBACK_GIZMO_HALF_M)
    this.handleMesh = new THREE.Mesh(new THREE.BoxGeometry(2, 2, 2), handleMat)
    this.handleMesh.renderOrder = 999
    this.handleMesh.scale.setScalar(FALLBACK_GIZMO_HALF_M)
    this.blueHandleMesh = new THREE.Mesh(new THREE.BoxGeometry(2, 2, 2), blueHandleMat)
    this.blueHandleMesh.renderOrder = 999
    this.blueHandleMesh.scale.setScalar(FALLBACK_GIZMO_HALF_M)

    const tileGeo = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(), new THREE.Vector3(), new THREE.Vector3(), new THREE.Vector3(),
    ])
    const tileMat = new THREE.LineBasicMaterial({ color: 0xffffff, depthTest: false, transparent: true, opacity: 0.75 })
    this.tileOutline = new THREE.LineLoop(tileGeo, tileMat)
    this.tileOutline.renderOrder = 998

    const refGeo = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(), new THREE.Vector3(), new THREE.Vector3(), new THREE.Vector3(),
    ])
    const refMat = new THREE.LineDashedMaterial({
      color: 0xffffff, depthTest: false, transparent: true, opacity: 0.4,
      dashSize: 0.1, gapSize: 0.06,
    })
    this.refOutline = new THREE.LineLoop(refGeo, refMat)
    this.refOutline.renderOrder = 997

    // Mesh order matters to tests: [0]=pin, [1]=green, [2]=blue.
    this.gizmoGroup.add(this.pinMesh)
    this.gizmoGroup.add(this.handleMesh)
    this.gizmoGroup.add(this.blueHandleMesh)
    this.gizmoGroup.add(this.tileOutline)
    this.gizmoGroup.add(this.refOutline)
  }

  /** Positions the pins at the tile's corners and refreshes the tile
   *  outline and the dashed 1× reference rectangle (all WORLD positions via
   *  `_localToWorld`). Scale of the boxes is owned by `updateGripScale`. */
  private _updateGizmo(local: Local2): void {
    if (
      this.pinMesh === null || this.handleMesh === null || this.blueHandleMesh === null ||
      this.tileOutline === null || this.refOutline === null
    ) return
    const s = this.session
    if (s === null) return
    const corners = this._cornersLocal(local)
    if (corners === null) {
      this.pinMesh.visible = false
      this.handleMesh.visible = false
      this.blueHandleMesh.visible = false
      this.tileOutline.visible = false
      this.refOutline.visible = false
      return
    }
    const { pin, green, blue } = corners
    this.pinMesh.visible = true
    this.handleMesh.visible = true
    this.blueHandleMesh.visible = true
    this.tileOutline.visible = true

    const toWorld = (p: readonly [number, number]): V3 =>
      this._localToWorld(local2ToWorld(s.anchor, s.uAx, s.vAx, p))
    const pinWorld = toWorld(pin)
    const greenWorld = toWorld(green)
    const blueWorld = toWorld(blue)
    // The tile's far corner (green + blue - pin in local coords — the
    // parallelogram closes exactly because the map is affine).
    const farWorld = toWorld([green[0] + blue[0] - pin[0], green[1] + blue[1] - pin[1]])

    this.pinMesh.position.set(pinWorld[0], pinWorld[1], pinWorld[2])
    this.handleMesh.position.set(greenWorld[0], greenWorld[1], greenWorld[2])
    this.blueHandleMesh.position.set(blueWorld[0], blueWorld[1], blueWorld[2])

    const tileAttr = this.tileOutline.geometry.getAttribute('position') as THREE.BufferAttribute
    tileAttr.setXYZ(0, pinWorld[0], pinWorld[1], pinWorld[2])
    tileAttr.setXYZ(1, greenWorld[0], greenWorld[1], greenWorld[2])
    tileAttr.setXYZ(2, farWorld[0], farWorld[1], farWorld[2])
    tileAttr.setXYZ(3, blueWorld[0], blueWorld[1], blueWorld[2])
    tileAttr.needsUpdate = true

    // The dashed 1× reference: the material's NATURAL tile, anchored at the
    // red pin, oriented to the tile's current U direction (handedness-
    // consistent perpendicular for V) — the fixed landmark that makes 1×
    // scale visible. Hidden only when the frame is too degenerate to have a
    // direction.
    const eu: [number, number] = [green[0] - pin[0], green[1] - pin[1]]
    const euLen = Math.hypot(eu[0], eu[1])
    const ev: [number, number] = [blue[0] - pin[0], blue[1] - pin[1]]
    const handed = eu[0] * ev[1] - eu[1] * ev[0] >= 0 ? 1 : -1
    if (euLen < 1e-12) {
      this.refOutline.visible = false
      return
    }
    this.refOutline.visible = true
    const uDir: [number, number] = [eu[0] / euLen, eu[1] / euLen]
    const vDir: [number, number] = handed === 1 ? [-uDir[1], uDir[0]] : [uDir[1], -uDir[0]]
    const refU: [number, number] = [pin[0] + uDir[0] * s.worldW, pin[1] + uDir[1] * s.worldW]
    const refV: [number, number] = [pin[0] + vDir[0] * s.worldH, pin[1] + vDir[1] * s.worldH]
    const refFar: [number, number] = [refU[0] + vDir[0] * s.worldH, refU[1] + vDir[1] * s.worldH]
    const refUW = toWorld(refU)
    const refVW = toWorld(refV)
    const refFarW = toWorld(refFar)
    const refAttr = this.refOutline.geometry.getAttribute('position') as THREE.BufferAttribute
    refAttr.setXYZ(0, pinWorld[0], pinWorld[1], pinWorld[2])
    refAttr.setXYZ(1, refUW[0], refUW[1], refUW[2])
    refAttr.setXYZ(2, refFarW[0], refFarW[1], refFarW[2])
    refAttr.setXYZ(3, refVW[0], refVW[1], refVW[2])
    refAttr.needsUpdate = true
    // LineDashedMaterial needs per-vertex line distances; keep the dash
    // pattern proportionate to the reference tile so it stays visible at
    // any material size.
    this.refOutline.computeLineDistances()
    const refMat = this.refOutline.material as THREE.LineDashedMaterial
    const perimeterScale = Math.max(s.worldW, s.worldH)
    refMat.dashSize = perimeterScale / 14
    refMat.gapSize = perimeterScale / 22
  }

  /**
   * Keeps the pin boxes a small, CONSTANT on-screen size regardless of
   * camera distance — called once per frame by the Viewport render loop
   * BEFORE `renderer.render()` (feature-detected via `'updateGripScale' in
   * tool`, same as ScaleTool). Also caches the `worldPerPixel` callback for
   * `_pickToleranceAt`/`_worldDragThresholdAt`, so pick and render agree.
   * The tile/reference outlines are world-sized on purpose — they ARE the
   * tile.
   *
   * The second parameter is a `worldPerPixel(dist)` CALLBACK derived from the
   * active `CameraRig`, not a viewport height: that is the shared,
   * projection-agnostic contract every other widget tool implements
   * (ScaleTool/RotateTool/SliceTool/ProtractorTool/SectionPlaneTool/
   * FollowMeTool) and the one the Viewport render loop actually passes. This
   * tool was written against an earlier `(camera, viewportHeightPx)` shape;
   * because the call site reaches the method through an `as` cast, the
   * mismatch could not be caught by the type checker, and the callback
   * arriving where a number was expected made every pin's scale NaN — three
   * then drew nothing at all, so the pins were invisible AND (via the same
   * cached values feeding `_pickToleranceAt`) unpickable.
   *
   * There is deliberately NO `instanceof PerspectiveCamera` guard: such a
   * guard silently hides every pin under parallel projection, which is the
   * precise bug `ScaleTool.updateGripScale`'s doc comment records having
   * already fixed once.
   */
  updateGripScale(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    if (this.pinMesh === null || this.handleMesh === null || this.blueHandleMesh === null) return
    this._pickWorldPerPixel = worldPerPixel

    for (const mesh of [this.pinMesh, this.handleMesh, this.blueHandleMesh]) {
      const dist = camera.position.distanceTo(mesh.position)
      const half = screenConstantWorldHalfFromWorldPerPixel(
        GIZMO_SCREEN_PX,
        worldPerPixel(dist),
        MIN_GIZMO_WORLD_HALF,
      )
      mesh.scale.setScalar(half)
    }
  }

  /** The pick tolerance (world units) at `pos`, matching what `pos` renders
   *  at on screen (same posture as `ScaleTool._pickToleranceAt`); fixed
   *  placeholder before the first render tick. */
  private _pickToleranceAt(ray: Ray, pos: V3): number {
    if (this._pickWorldPerPixel === null) {
      return FALLBACK_GIZMO_HALF_M * GIZMO_PICK_MULTIPLIER
    }
    const dist = dist3(ray.origin, pos)
    const half = screenConstantWorldHalfFromWorldPerPixel(
      GIZMO_SCREEN_PX,
      this._pickWorldPerPixel(dist),
      MIN_GIZMO_WORLD_HALF,
    )
    return half * GIZMO_PICK_MULTIPLIER
  }

  private _clearGizmo(): void {
    const disposables: Array<THREE.Mesh | THREE.LineLoop | null> = [
      this.pinMesh, this.handleMesh, this.blueHandleMesh, this.tileOutline, this.refOutline,
    ]
    for (const obj of disposables) {
      if (obj === null) continue
      this.gizmoGroup.remove(obj)
      obj.geometry.dispose()
      ;(obj.material as THREE.Material).dispose()
    }
    this.pinMesh = null
    this.handleMesh = null
    this.blueHandleMesh = null
    this.tileOutline = null
    this.refOutline = null
  }

  // ─────────────────────────────────────────── typed absolute values

  /**
   * REFUSAL RULE for the typed-entry state machine: a typed value that
   * cannot be applied must NEVER be swallowed silently — the Enter handler
   * only falls through to the session-level commit when the buffer is
   * empty, so a refusal that leaves the buffer populated would wedge Enter
   * permanently (every later press re-enters the same dead branch). Every
   * refusal therefore toasts WHY, clears the buffer, and restores the live
   * readout; the session (and any held grab) stays open. The only early
   * returns exempt from this rule are `_abortIfHistoryMoved` (which toasts
   * and closes the whole session itself, buffer included) and the
   * unreachable no-session guard.
   *
   * BLAST RADIUS: refusals are reachable ONLY from `onKey`'s Enter branch
   * with a non-empty buffer — the user explicitly asking for the typed
   * value. Buffer-editing keystrokes (digits, `x`, `.`, `-`, Backspace,
   * Tab) never refuse, so no toast can fire mid-typing or under digit
   * autorepeat; the Viewport's per-keystroke pointer-move replays enter
   * through `onPointerMove`, which has no refusal path at all. A held
   * (autorepeating) Enter on a bad buffer toasts once, then the cleared
   * buffer lets the next repeat reach the ordinary session close.
   */
  private _refuseTyped(message: string): void {
    const s = this.session
    this.onToast(message)
    this._resetTyped()
    if (s !== null && s.grab !== null && s.grab.kind === 'translate') {
      this._reportTypedOrLiveTranslate(s.grab)
    } else {
      this._reportIdleAbsolute()
    }
  }

  /**
   * Applies an EXACT ABSOLUTE typed value — rotation degrees or scale
   * factor for green, skew degrees or V-scale for blue — about the red
   * pin, from the CURRENT live frame (`workingLocal`): typing `45` puts
   * the texture at 45° from the face's own axes no matter what any drag
   * did first, and the quantity NOT being typed keeps its current live
   * value. Routes through the exact same `scaleRotateLocal`/
   * `shearScaleLocal` a drag uses, via a synthetic dragged-to corner.
   * Ends any held grab like a release; the session stays open (a further
   * Enter commits). Every refusal follows `_refuseTyped`'s rule.
   */
  private _applyTypedAbsolute(kind: 'scaleRotate' | 'shear'): void {
    const s = this.session
    if (s === null) return // unreachable via onKey; nothing typed to refuse
    if (this._abortIfHistoryMoved()) return // toasts + closes the session itself
    const parsed = parseScaleOrRotate(this.typed, this.typedMode)
    if (parsed === null) {
      this._refuseTyped('Could not read that value — type degrees, or a scale like 2x')
      return
    }
    if (parsed.mode === 'scale' && !(parsed.value > 0)) {
      this._refuseTyped('Scale must be greater than zero')
      return
    }

    const local = s.workingLocal
    const corners = this._cornersLocal(local)
    const abs = frameAbsolute(local, s.worldW, s.worldH)
    if (corners === null || abs === null) {
      this._refuseTyped('The texture frame is degenerate — drag a pin to rebuild it first')
      return
    }
    const { pin, green } = corners

    let result: Local2 | null = null
    if (kind === 'scaleRotate') {
      const eu: [number, number] = [green[0] - pin[0], green[1] - pin[1]]
      const kTarget = parsed.mode === 'scale' ? parsed.value : abs.scaleU
      const thetaTarget = parsed.mode === 'rotate' ? parsed.value / DEG : abs.angle
      const ratio = kTarget / abs.scaleU
      const rotated = matVec(rotation2(thetaTarget - abs.angle), eu)
      const newHandle: [number, number] = [pin[0] + rotated[0] * ratio, pin[1] + rotated[1] * ratio]
      result = scaleRotateLocal(local, pin, green, newHandle)
    } else {
      const eu: [number, number] = [green[0] - pin[0], green[1] - pin[1]]
      const euLen = Math.hypot(eu[0], eu[1])
      if (euLen < 1e-12) {
        this._refuseTyped('The texture frame is degenerate — drag a pin to rebuild it first')
        return
      }
      const uDir: [number, number] = [eu[0] / euLen, eu[1] / euLen]
      const perp: [number, number] = abs.handed === 1 ? [-uDir[1], uDir[0]] : [uDir[1], -uDir[0]]
      const kvTarget = parsed.mode === 'scale' ? parsed.value : abs.scaleV
      const skewTarget = parsed.mode === 'rotate' ? parsed.value / DEG : abs.skew
      // `signedAngle2` is a plain CCW-signed angle, so reconstruction is
      // always a CCW rotation of the (handedness-consistent) perpendicular,
      // mirrored frames included: R(skew)·perp ∥ E_v by definition.
      const dir = matVec(rotation2(skewTarget), perp)
      const evLen = kvTarget * s.worldH
      const newHandle: [number, number] = [pin[0] + dir[0] * evLen, pin[1] + dir[1] * evLen]
      result = shearScaleLocal(local, pin, newHandle)
    }
    if (result === null) {
      this._refuseTyped('That value would collapse the texture tile')
      return
    }

    this._applyPreview(result)
    if (s.grab !== null) s.grab = null
    this._resetTyped()
    this._reportIdleAbsolute()
  }

  /**
   * Applies an EXACT typed length to the in-progress translate grab,
   * ending it like a release — measured in document units along the drag's
   * own established direction (`lastDelta`), in TRUE world meters
   * (`_worldLen`). A grab with no established direction yet cannot apply a
   * length, and refuses per `_refuseTyped`'s rule (toast + clear — never a
   * silent swallow that would wedge Enter).
   */
  private _commitTypedTranslate(grab: Extract<Grab, { kind: 'translate' }>): void {
    if (this._abortIfHistoryMoved()) return // toasts + closes the session itself
    const meters = parseLengthToMeters(this.typed)
    if (meters === null) {
      this._refuseTyped('Could not read that length')
      return
    }
    const dirWorldLen = this._worldLen(grab.lastDelta)
    if (dirWorldLen < 1e-9) {
      this._refuseTyped('Drag first to set a direction — a typed length moves along it')
      return
    }

    const scale = meters / dirWorldLen
    const syntheticDelta: [number, number] = [grab.lastDelta[0] * scale, grab.lastDelta[1] * scale]
    this._applyPreview(translateLocal(grab.baseLocal, syntheticDelta))
    const s = this.session
    if (s !== null && s.grab !== null) s.grab = null
    this._resetTyped()
    this._reportIdleAbsolute()
  }

  // ─────────────────────────────────────────────────────────── readouts

  /** The typed buffer (tagged by mode) while one is in progress, else the
   *  CURRENT ABSOLUTE readout for `kind` — always measured against the
   *  face axes and the natural tile size, never a drag's start. */
  private _reportTypedOrLiveAbsolute(kind: 'scaleRotate' | 'shear'): void {
    if (this.typed !== '') {
      const xMatch = /^(-?(?:\d+\.?\d*|\.\d+))x$/i.exec(this.typed)
      if (xMatch !== null) {
        this.onMeasurementCb(`×${xMatch[1]}`)
        return
      }
      this.onMeasurementCb(this.typedMode === 'rotate' ? `${this.typed}°` : `×${this.typed}`)
      return
    }
    this._reportAbsolute(kind)
  }

  /** The current ABSOLUTE state: `×scaleU angle°` for green, `×scaleV
   *  skew°` for blue — from the live frame, so "am I at 1× / 45° yet" is
   *  always answerable at a glance. */
  private _reportAbsolute(kind: 'scaleRotate' | 'shear'): void {
    const s = this.session
    if (s === null) return
    const abs = frameAbsolute(s.workingLocal, s.worldW, s.worldH)
    if (abs === null) {
      this.onMeasurementCb('')
      return
    }
    if (kind === 'shear') {
      this.onMeasurementCb(`×${abs.scaleV.toFixed(2)}  ${(abs.skew * DEG).toFixed(1)}° skew`)
    } else {
      this.onMeasurementCb(`×${abs.scaleU.toFixed(2)}  ${(abs.angle * DEG).toFixed(1)}°`)
    }
  }

  /** The idle-session readout — the green (angle/scale) absolute state, or
   *  blue's when it is the selected typed target. */
  private _reportIdleAbsolute(): void {
    this._reportAbsolute(this._typedKind())
  }

  /** Typed-or-live readout for the translate (red) grab. */
  private _reportTypedOrLiveTranslate(grab: Extract<Grab, { kind: 'translate' }>): void {
    if (this.typed !== '') {
      this.onMeasurementCb(typedReadout(this.typed))
      return
    }
    this._reportLiveTranslate(grab)
  }

  /** The live distance moved so far (TRUE world length), formatted in the
   *  document's current unit — empty when nothing has moved yet. */
  private _reportLiveTranslate(grab: Extract<Grab, { kind: 'translate' }>): void {
    const dist = this._worldLen(grab.lastDelta)
    if (dist < 1e-9) {
      this.onMeasurementCb('')
      return
    }
    this.onMeasurementCb(formatLength(dist))
  }
}
