/**
 * WalkTool — Camera ▸ Walk (docs/design/camera.md §4): the third
 * walkthrough camera tool. Drag from the press point: vertical delta walks
 * forward/back along the HORIZONTAL forward direction, horizontal delta
 * turns (yaw); holding Shift swaps the mapping to strafe (horizontal) and
 * eye-height change (vertical) — SketchUp's own Walk modifier. Eye height is
 * maintained above the ground plane (Z=0) at all times except during a
 * Shift drag, matching the flat-ground assumption this v1 slice makes
 * explicit (no collision/gravity against geometry — a documented scope
 * cut, `pointerRawMove`'s doc + the design).
 *
 * Like Look Around, this is a pure screen-space drag with no "point in the
 * scene" involved, so it drives its gesture off `onPointerRawDown`/
 * `onPointerRawMove` (raw canvas pixels) rather than `onPointerMove`'s
 * `Snap`/`Ray`. Unlike Look Around's INCREMENTAL accumulation, Walk's speed
 * is proportional to net drag distance FROM THE PRESS POINT (SketchUp's own
 * feel: hold and drag further from where you clicked to walk faster) — see
 * `walkDrag`'s doc. The whole press-to-release drag is computed fresh every
 * move from one fixed `baseEye`/`baseYaw`/`baseHeight` (the pose at the
 * START of the current drag, or the tool's construction-time pose before
 * any drag); only a real release COMMITS the live preview as the new base,
 * ready for the next press.
 */

import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import { getLengthUnit } from '../settings/units'
import { eyeHeightCapturesKey, eyeHeightHandleKey } from './cameraEyeHeightVcb'
import { headingFromForward, forwardFromYawPitch, walkDrag, horizontalBasis, type V3 } from './cameraWalkMath'

/** Turn rate: radians of yaw per CSS pixel of horizontal drag (non-Shift). */
export const WALK_TURN_RAD_PER_PIXEL = 0.0025
/** Move rate: meters per CSS pixel of drag (forward/back, strafe, or
 * height, whichever `walkDrag` selects). A ~200px drag walks ~4 m — brisk
 * but controllable. */
export const WALK_MOVE_M_PER_PIXEL = 0.02

export type GetEye = () => V3
export type GetForward = () => V3
export type GetEyeHeight = () => number
export type SetEyeHeight = (height: number) => void
export type ApplyPose = (eye: V3, forward: V3) => void
export type OnMeasurement = (text: string) => void

export class WalkTool implements Tool {
  readonly name = 'Walk'

  private baseEye: V3
  private baseYaw: number
  private baseHeight: number
  /** Mirrors whatever was last applied — `onPointerUp` commits THIS (not a
   * pixel recompute) as the new base, since the browser's pointerup does
   * not itself carry a fresh pixel position through the `Tool` interface. */
  private liveEye: V3
  private liveYaw: number
  private liveHeight: number
  private pressPx = 0
  private pressPy = 0
  private dragging = false
  /** The Shift state the CURRENT drag segment's `pressPx`/`pressPy` anchor
   * was taken under — see `onPointerRawMove`'s re-anchor branch. Only
   * meaningful once `sampledShift` is true (`onPointerRawDown` doesn't
   * carry modifier state, so there is no correct value to seed it with
   * before the drag's first move sample). */
  private shiftActive = false
  private sampledShift = false
  private typed = ''

  constructor(
    getEye: GetEye,
    getForward: GetForward,
    private getEyeHeight: GetEyeHeight,
    private setEyeHeight: SetEyeHeight,
    private applyPose: ApplyPose,
    private onEscapeToSelect: () => void,
    private onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.baseYaw = headingFromForward(getForward())
    this.baseHeight = getEyeHeight()
    const eye = getEye()
    // Eye height maintained above the ground plane (design §4) — snap the
    // seeded eye onto the shared session height immediately, so entering
    // Walk from a tilted Position-Camera/Look-Around pose levels out.
    this.baseEye = [eye[0], eye[1], this.baseHeight]
    this.liveEye = this.baseEye
    this.liveYaw = this.baseYaw
    this.liveHeight = this.baseHeight
  }

  statusHint(): string {
    return 'Drag up/down to walk forward/back, left/right to turn. Hold Shift to strafe and change height. Esc returns to Select.'
  }

  // Not used for this tool's own gesture (see the module doc), but the
  // Viewport still resolves and passes a snap/ray for the shared cue layer.
  onPointerMove(_snap: Snap | null, _ray: Ray): void { /* no-op */ }
  onPointerDown(_snap: Snap | null, _ray: Ray): void { /* no-op — real gesture is in the raw hooks */ }

  onPointerRawDown(xPx: number, yPx: number): void {
    this.pressPx = xPx
    this.pressPy = yPx
    this.dragging = true
    // No modifier state reaches us here (see `sampledShift`'s doc) — the
    // first move of the drag establishes it below without re-anchoring.
    this.sampledShift = false
  }

  onPointerRawMove(xPx: number, yPx: number, buttons: number, mods: { shift: boolean }): void {
    if ((buttons & 1) === 0) {
      this.dragging = false
      return
    }
    if (!this.dragging) {
      // Button held but no press event reached us first (shouldn't normally
      // happen) — treat this sample as a fresh press so the drag still works.
      this.pressPx = xPx
      this.pressPy = yPx
      this.dragging = true
      this.shiftActive = mods.shift
      this.sampledShift = true
      return
    }
    if (!this.sampledShift) {
      // The drag's first move sample: nothing to re-anchor against yet —
      // `pressPx`/`pressPy` (from onPointerRawDown) are the correct anchor
      // for whatever mapping is in effect from the start of this drag.
      this.shiftActive = mods.shift
      this.sampledShift = true
    } else if (mods.shift !== this.shiftActive) {
      // Shift flips `walkDrag`'s mapping wholesale — the SAME accumulated
      // dx/dy since press means "turn/walk forward" one moment and
      // "strafe/change height" the next. `dx`/`dy` are cumulative since
      // `pressPx`/`pressPy` (by design — Walk's speed is proportional to
      // net drag distance, not per-frame velocity), so reinterpreting them
      // under the new mapping without resetting the anchor reproduces
      // however far the pointer had ALREADY travelled under the OLD
      // mapping as a sudden jump under the new one (e.g. a long forward
      // walk's accumulated `dy` instantly reread as a multi-metre height
      // change the instant Shift is pressed) — playtest finding 3, the
      // camera "jumping a large random-looking distance" mid-gesture.
      // Re-anchor from the CURRENT live pose/pointer position exactly as a
      // release+re-press would, so the new mapping starts at a zero delta
      // and grows smoothly from here.
      this.baseEye = this.liveEye
      this.baseYaw = this.liveYaw
      this.baseHeight = this.liveHeight
      this.pressPx = xPx
      this.pressPy = yPx
      this.shiftActive = mods.shift
    }
    const dx = xPx - this.pressPx
    const dy = yPx - this.pressPy
    const delta = walkDrag(dx, dy, WALK_TURN_RAD_PER_PIXEL, WALK_MOVE_M_PER_PIXEL, mods.shift)
    const yaw = this.baseYaw + delta.yawDeltaRad
    const { forward, right } = horizontalBasis(yaw)
    const height = this.baseHeight + delta.heightDeltaM
    const eye: V3 = [
      this.baseEye[0] + forward[0] * delta.forwardDeltaM + right[0] * delta.strafeDeltaM,
      this.baseEye[1] + forward[1] * delta.forwardDeltaM + right[1] * delta.strafeDeltaM,
      height,
    ]
    this.liveEye = eye
    this.liveYaw = yaw
    this.liveHeight = height
    this.applyPose(eye, forwardFromYawPitch(yaw, 0))
  }

  onPointerUp(_snap: Snap | null, _ray: Ray): void {
    this._commit()
  }

  /** Commits the live drag preview as the new base pose, and persists the
   * (possibly Shift-changed) height to the session-shared value so
   * Position Camera/Look Around pick it up too. */
  private _commit(): void {
    if (!this.dragging) return
    this.dragging = false
    this.baseEye = this.liveEye
    this.baseYaw = this.liveYaw
    this.baseHeight = this.liveHeight
    if (this.baseHeight !== this.getEyeHeight()) this.setEyeHeight(this.baseHeight)
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      this.onEscapeToSelect()
      return
    }
    const { typed, committed, readout } = eyeHeightHandleKey(this.typed, ev.key, getLengthUnit())
    this.typed = typed
    if (committed !== null) {
      this.setEyeHeight(committed)
      this.baseHeight = committed
      this.baseEye = [this.baseEye[0], this.baseEye[1], committed]
      this.liveEye = this.baseEye
      this.liveHeight = committed
      this.applyPose(this.baseEye, forwardFromYawPitch(this.baseYaw, 0))
    }
    this.onMeasurement(readout)
  }

  capturingInput(): boolean {
    return this.typed !== ''
  }

  capturesKey(key: string): boolean {
    return eyeHeightCapturesKey(this.typed, key)
  }

  cancel(): void {
    this.dragging = false
    this.typed = ''
    this.onMeasurement('')
  }
}
