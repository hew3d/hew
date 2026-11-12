/**
 * planePin — the Shift-pinned drawing plane the draw tools (Line/Rectangle/
 * Circle/Polygon/Arc) share (GitHub issue 14; the Getting Started guide's
 * "hover over a face that is already on your intended plane and press
 * Shift to lock the cursor into that plane").
 *
 * While a draw tool is IDLE, every pointer move records the plane the next
 * click would land on — the hovered eligible face's plane, a hovered
 * sketch's plane, or the ground (`trackHover`). A Shift press pins that
 * plane: the tool's snap query is constrained to it, the first click
 * anchors a PLANE-mode gesture on its orientation (through a named point
 * off the plane when the click is on one, `clickTarget`), and the cursor
 * stays on it wherever it goes — off the face's edge, over the ground, over another solid.
 * Pressing Shift again, or Escape while idle, releases the pin. The pin
 * survives a completed gesture (draw several shapes on one plane) exactly
 * like the arrow-key idle plane lock does, and `cancel()` — a tool switch,
 * a document reset — drops it.
 *
 * Toggle, not hold: the same convention as Rotate/Protractor/Slice ("press
 * Shift again to release"), and the only one that works for a two-click
 * gesture the user cannot keep a modifier held through comfortably.
 * `setShiftHeld(true)` arrives on every keydown AUTOREPEAT while Shift is
 * held, so the toggle fires once per physical press: a repeat while the key
 * is still down is ignored, and the release re-arms it.
 *
 * The pinned plane is a fixed plane in space, not an axis through the
 * click (the arrow-key lock's shape), so the two are mutually exclusive:
 * pinning clears an arrow lock and an arrow lock clears the pin — the tool
 * owns that pairing, this class only owns the pin.
 */

import type { V3 } from '../viewport/geoHelpers'
import type { DrawPlane } from './drawPlane'
import type { SketchTarget } from './sketchGesture'
import type { EditContext, Snap } from './types'
import { instanceOf, isGroundPlane, isNamedPointSnap } from './drawPlane'
import { distanceOffPlane, FACE_PLANE_EPS_M } from './faceDraw'

/** A plane worth pinning, with the hover point it was found under (the
 *  point the plane cue draws through until the cursor moves again). */
export interface PinnedPlane {
  plane: DrawPlane
  through: V3
}

export class PlanePin {
  private pinned: PinnedPlane | null = null
  private hover: PinnedPlane | null = null
  /** Whether Shift is physically down — the keydown-autorepeat guard. */
  private shiftDown = false

  /** The active pin, or null. */
  get current(): PinnedPlane | null {
    return this.pinned
  }

  /** Record the plane an idle click would land on right now, under the
   *  cursor at `through`. Ignored while a pin is active (the pin IS the
   *  plane; there is nothing to track), so releasing the pin never
   *  re-pins a stale hover — the next pointer move records a fresh one. */
  trackHover(plane: DrawPlane, through: V3): void {
    if (this.pinned !== null) return
    this.hover = { plane, through }
  }

  /**
   * Shift pressed (`held`) or released. One physical press toggles: pins
   * the tracked hover plane, or releases the active pin. Returns whether
   * the pin state changed, so the tool can drop a competing arrow lock and
   * the host can refresh its status hint and plane cue.
   */
  setShiftHeld(held: boolean): boolean {
    if (!held) {
      this.shiftDown = false
      return false
    }
    if (this.shiftDown) return false // keydown autorepeat
    this.shiftDown = true
    return this.toggle()
  }

  /**
   * What one Shift press does, without the autorepeat guard: pin the
   * tracked hover plane, or release the active pin. The view die's custom
   * pip calls this through the tool (a click has no physical Shift to
   * guard). Returns whether the pin state changed.
   */
  toggle(): boolean {
    if (this.pinned !== null) {
      this.pinned = null
      this.hover = null
      return true
    }
    if (this.hover === null) return false
    this.pinned = this.hover
    return true
  }

  /** Pin `plane` outright (the view die's custom pip pins its candidate,
   *  which need not be the tracked hover). */
  pinPlane(plane: PinnedPlane): void {
    this.pinned = plane
  }

  /** The plane a Shift press would pin right now (the last tracked hover),
   *  or null before any hover, under a pin, or after a release. */
  get tracked(): PinnedPlane | null {
    return this.hover
  }

  /**
   * Record Shift's physical state WITHOUT toggling — for the moments a tool
   * routes Shift elsewhere (a mid-chain axis lock). Without this, a chain
   * finished with Shift still held (lock the axis, click the end point,
   * release later — the SketchUp habit) let the key's autorepeat reach an
   * IDLE tool with the guard unset, and the first repeat silently pinned
   * whatever plane the last hover recorded — often the ground — so the next
   * line started "on the ground" with every chip reading "projected".
   */
  markShift(held: boolean): void {
    this.shiftDown = held
  }

  /** Drop the pin (Escape while idle, an arrow lock, `cancel()`). The
   *  Shift-down guard is left alone: a release still has to arrive. */
  clear(): void {
    this.pinned = null
    this.hover = null
  }

  /** Put a pin back after a `cancel()` that had to run for other reasons —
   *  Escape aborting an in-progress gesture keeps the pin exactly as it
   *  keeps the arrow-key lock (an idle aiming choice, not gesture state). */
  restore(pinned: PinnedPlane | null): void {
    this.pinned = pinned
  }

  /** The plane-mode target a first click anchors onto while pinned, or
   *  null when nothing is pinned. Namespaced to the edit context's
   *  instance exactly like every other plane-mode target.
   *
   *  A pin holds the plane's ORIENTATION for the first click (R18 c): when
   *  `snap` names a point (corner, midpoint, center, quadrant, guide point)
   *  off the pinned plane, the target is the pinned plane's orientation
   *  THROUGH that point, so the gesture starts at the point the cursor named
   *  instead of on its foot. Later points of the gesture are held to the
   *  frozen plane and project, disclosed, as before. */
  clickTarget(editContext: EditContext, snap: Snap | null = null): { plane: DrawPlane; target: SketchTarget } | null {
    if (this.pinned === null) return null
    const plane = this.pointPlane(snap) ?? this.pinned.plane
    return { plane, target: { kind: 'plane', plane, instance: instanceOf(editContext) } }
  }

  /** The pinned plane's orientation through `snap`'s point, when `snap` is a
   *  named point more than the plane tolerance off the pinned plane; null
   *  otherwise (nothing pinned, no named point, or the point is on it). */
  pointPlane(snap: Snap | null): DrawPlane | null {
    if (this.pinned === null || !isNamedPointSnap(snap)) return null
    const pinned = this.pinned.plane
    const point: V3 = [snap.x, snap.y, snap.z]
    if (Math.abs(distanceOffPlane(point, pinned.origin, pinned.normal)) <= FACE_PLANE_EPS_M) return null
    return { ...pinned, origin: point, ground: isGroundPlane(point, pinned.normal) }
  }

  /** The snap constraint the pinned plane imposes: candidates are held to
   *  the plane, so an edge or face off it is not a target, its foot on the
   *  plane is. Named points (corner, midpoint, center, quadrant, guide
   *  point) off the plane are still offered (`offPlanePoints`): the first
   *  click starts through them (`clickTarget`), and later clicks drop them
   *  onto the frozen plane, disclosed (`cursorOnFrozenPlane`). Null when
   *  nothing is pinned. */
  constraint(): { constraintPlane: { point: V3; normal: V3 }; offPlanePoints: true } | null {
    if (this.pinned === null) return null
    return { constraintPlane: { point: this.pinned.plane.origin, normal: this.pinned.plane.normal }, offPlanePoints: true }
  }
}
