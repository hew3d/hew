/**
 * PositionCameraTool — Camera ▸ Position Camera (docs/design/camera.md §4):
 * the first of the three real "walkthrough" camera tool classes (Orbit/Pan/
 * Zoom stay mouseButtons remaps on `OrbitControls`, never real `Tool`s).
 *
 * Gesture: a plain CLICK places the eye `eyeHeight` above the clicked point,
 * looking horizontally along whatever heading the camera had just before the
 * click (`positionCameraClick`); a DRAG places the eye above the PRESS point
 * and looks toward the RELEASE point in true 3D — not flattened to
 * horizontal, so dragging onto a raised feature looks AT it
 * (`positionCameraDrag`). Both endings auto-switch the active tool to Look
 * Around (SketchUp's own behavior) via `onAutoSwitchToLookAround`.
 *
 * Click vs. drag is decided at `onPointerUp` (the `Tool` interface's one
 * genuine release hook — see its doc comment) by comparing the net
 * WORLD-space movement since the press against
 * `POSITION_CAMERA_DRAG_THRESHOLD_M`, the same idiom `SectionPlaneTool`'s
 * `OFFSET_DRAG_THRESHOLD_M` uses (`Tool` hands gestures a `Ray`, not screen
 * pixels, so a pixel threshold is unavailable here). `onPointerMove` while
 * pressed applies a LIVE preview via the drag math (so the view visibly
 * tracks the cursor while dragging); `onPointerUp` recomputes the FINAL pose
 * fresh (click or drag) from the press/release points themselves, mirroring
 * `SectionPlaneTool._commitOffsetOrToggle`'s "re-derive at commit" — a
 * degenerate near-zero-movement release must still resolve to the CLICK
 * pose (horizontal, pre-click heading), not the drag math's literal
 * straight-down look for `press == release`.
 *
 * Eye height (VCB, `cameraEyeHeightVcb.ts`) is session-shared: the
 * `getEyeHeight`/`setEyeHeight` callbacks read/write a value Viewport.tsx
 * keeps in a closure variable spanning all three camera tools, per design
 * §4 ("the value persists for the session and is shared with Walk/Look
 * Around").
 */

import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import { getLengthUnit } from '../settings/units'
import { eyeHeightCapturesKey, eyeHeightHandleKey } from './cameraEyeHeightVcb'
import { positionCameraClick, positionCameraDrag, type V3 } from './cameraWalkMath'

/** Net world-space movement (meters) since the press that turns a release
 * into a genuine DRAG rather than a plain CLICK — comfortably past
 * float/ray noise on a static click while far below any deliberate drag
 * (mirrors `SectionPlaneTool.OFFSET_DRAG_THRESHOLD_M`'s reasoning, scaled
 * up: a camera placement drag is a much coarser gesture than a sweep). */
export const POSITION_CAMERA_DRAG_THRESHOLD_M = 0.05

export type GetForward = () => V3
export type GetEyeHeight = () => number
export type SetEyeHeight = (height: number) => void
export type ApplyPose = (eye: V3, forward: V3) => void
export type OnMeasurement = (text: string) => void

function toV3(snap: Snap): V3 {
  return [snap.x, snap.y, snap.z]
}

function dist(a: V3, b: V3): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])
}

export class PositionCameraTool implements Tool {
  readonly name = 'Position Camera'

  private pressPoint: V3 | null = null
  /** The camera's forward direction snapshotted at press — the heading a
   * plain click preserves (design §4). */
  private preClickForward: V3 = [1, 0, 0]
  private typed = ''

  constructor(
    private getForward: GetForward,
    private getEyeHeight: GetEyeHeight,
    private setEyeHeight: SetEyeHeight,
    private applyPose: ApplyPose,
    private onAutoSwitchToLookAround: () => void,
    private onEscapeToSelect: () => void,
    private onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {}

  statusHint(): string {
    return 'Click to stand there, facing your current view — or drag to look toward where you release. Type a height + Enter to set eye height. Esc returns to Select.'
  }

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (this.pressPoint === null || snap === null) return
    const { eye, forward } = positionCameraDrag(this.pressPoint, toV3(snap), this.getEyeHeight(), this.preClickForward)
    this.applyPose(eye, forward)
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return
    this.pressPoint = toV3(snap)
    this.preClickForward = this.getForward()
    const { eye, forward } = positionCameraClick(this.pressPoint, this.getEyeHeight(), this.preClickForward)
    this.applyPose(eye, forward)
  }

  onPointerUp(snap: Snap | null, _ray: Ray): void {
    if (this.pressPoint === null) return
    const press = this.pressPoint
    this.pressPoint = null
    const release = snap !== null ? toV3(snap) : press
    const eyeHeight = this.getEyeHeight()
    const { eye, forward } =
      dist(press, release) < POSITION_CAMERA_DRAG_THRESHOLD_M
        ? positionCameraClick(press, eyeHeight, this.preClickForward)
        : positionCameraDrag(press, release, eyeHeight, this.preClickForward)
    this.applyPose(eye, forward)
    this.onAutoSwitchToLookAround()
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      this.onEscapeToSelect()
      return
    }
    const { typed, committed, readout } = eyeHeightHandleKey(this.typed, ev.key, getLengthUnit())
    this.typed = typed
    if (committed !== null) this.setEyeHeight(committed)
    this.onMeasurement(readout)
  }

  capturingInput(): boolean {
    return this.typed !== ''
  }

  capturesKey(key: string): boolean {
    return eyeHeightCapturesKey(this.typed, key)
  }

  cancel(): void {
    this.pressPoint = null
    this.typed = ''
    this.onMeasurement('')
  }
}
