/**
 * LookAroundTool — Camera ▸ Look Around (docs/design/camera.md §4): the
 * second walkthrough camera tool. A drag yaws/pitches the view about the
 * fixed eye position (pitch clamped ±`PITCH_CLAMP_DEG`, up stays +Z); the
 * VCB is the eye height, and typing a new one re-heights the eye IN PLACE
 * (same x/y, new z) without otherwise changing the look direction.
 *
 * Unlike Position Camera, this gesture has no notion of "a point in the
 * scene" at all — it is a pure screen-space mouse-look, so it uses the
 * `onPointerRawMove` hook (raw canvas-pixel deltas) rather than the usual
 * `Snap`/`Ray`-based `onPointerMove` (see that hook's doc in `types.ts`).
 * Yaw/pitch accumulate INCREMENTALLY: each call advances the tool's own
 * running `yaw`/`pitch` state by the delta since the PREVIOUS raw-move call
 * (not since the press), tracked via `lastPx`/`lastPy` — reset to `null`
 * whenever the left button is not held, so a fresh press starts a fresh
 * delta baseline instead of jumping by the gap since the last drag ended.
 *
 * `yaw`/`pitch` are seeded from the CURRENT camera pose at construction
 * (via `getForward`), so entering Look Around — whether by menu/palette or
 * auto-switched from Position Camera — always continues smoothly from
 * wherever the view already was, with no jump.
 */

import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import { getLengthUnit } from '../settings/units'
import { eyeHeightCapturesKey, eyeHeightHandleKey } from './cameraEyeHeightVcb'
import { headingFromForward, forwardFromYawPitch, lookAroundDrag, type V3 } from './cameraWalkMath'

/** Mouse-look sensitivity: radians of yaw/pitch per CSS pixel of drag.
 * Arbitrary but small enough that a full viewport-width drag (~1000px)
 * sweeps a bit over a full turn (~2.5 rad) — a brisk but controllable rate. */
export const LOOK_AROUND_RAD_PER_PIXEL = 0.0025

export type GetEye = () => V3
export type GetForward = () => V3
export type GetEyeHeight = () => number
export type SetEyeHeight = (height: number) => void
export type ApplyPose = (eye: V3, forward: V3) => void
export type OnMeasurement = (text: string) => void

/** Extracts the pitch (radians above horizontal) from an arbitrary (not
 * necessarily unit) forward vector — the sibling of `headingFromForward`
 * this module needs to seed `pitch` from the pre-activation camera pose. */
function pitchFromForward(forward: V3): number {
  const len = Math.hypot(forward[0], forward[1], forward[2])
  if (len < 1e-9) return 0
  return Math.asin(Math.min(1, Math.max(-1, forward[2] / len)))
}

export class LookAroundTool implements Tool {
  readonly name = 'Look Around'

  private eye: V3
  private yaw: number
  private pitch: number
  private lastPx: number | null = null
  private lastPy: number | null = null
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
    this.eye = getEye()
    const forward = getForward()
    this.yaw = headingFromForward(forward)
    this.pitch = pitchFromForward(forward)
  }

  statusHint(): string {
    return 'Drag to look around. Type a height + Enter to re-height the eye in place. Esc returns to Select.'
  }

  // Not used for this tool's own gesture (see the module doc), but the
  // Viewport still resolves and passes a snap/ray for the shared cue layer.
  onPointerMove(_snap: Snap | null, _ray: Ray): void { /* no-op */ }
  onPointerDown(_snap: Snap | null, _ray: Ray): void { /* no-op — this tool has no click gesture */ }

  /** Seeds the delta baseline AT THE PRESS itself, not on the first
   * `onPointerRawMove` sample after it — a press-then-immediately-drag-fast
   * would otherwise lose whatever distance the pointer covered between the
   * press and that first move sample (worse under a browser's own pointer-
   * event coalescing, which can deliver a fast synthetic/programmatic drag
   * as a single move event; see `WalkTool.onPointerRawDown`, the same fix). */
  onPointerRawDown(xPx: number, yPx: number): void {
    this.lastPx = xPx
    this.lastPy = yPx
  }

  onPointerRawMove(xPx: number, yPx: number, buttons: number, _mods: { shift: boolean }): void {
    if ((buttons & 1) === 0) {
      // Not dragging: reset the delta baseline so the next press starts
      // fresh instead of jumping by however far the pointer drifted while up.
      this.lastPx = null
      this.lastPy = null
      return
    }
    if (this.lastPx === null || this.lastPy === null) {
      this.lastPx = xPx
      this.lastPy = yPx
      return
    }
    const dx = xPx - this.lastPx
    const dy = yPx - this.lastPy
    this.lastPx = xPx
    this.lastPy = yPx
    const { yawRad, pitchRad } = lookAroundDrag(this.yaw, this.pitch, dx, dy, LOOK_AROUND_RAD_PER_PIXEL)
    this.yaw = yawRad
    this.pitch = pitchRad
    this.applyPose(this.eye, forwardFromYawPitch(this.yaw, this.pitch))
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
      // Re-height IN PLACE: same x/y, new z (design §4) — the eye-height
      // convention every one of the three tools shares (an absolute Z
      // above the ground plane, not an offset from wherever the eye
      // happens to be right now).
      this.eye = [this.eye[0], this.eye[1], committed]
      this.applyPose(this.eye, forwardFromYawPitch(this.yaw, this.pitch))
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
    this.lastPx = null
    this.lastPy = null
    this.typed = ''
    this.onMeasurement('')
  }
}
