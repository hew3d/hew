/**
 * cameraWalkMath — pure helpers for Position Camera / Look Around / Walk
 * (docs/design/camera.md §4). No three.js or DOM imports — fully testable
 * in Node/vitest, mirroring `moveInput.ts`/`transformMath.ts`'s split
 * between pure gesture math and the thin `Tool` classes that wrap it.
 *
 * World convention throughout this module (matching the rest of the app):
 * +Z is up, the ground plane is Z=0, and a "heading"/"yaw" is the compass
 * angle in the XY plane measured from +X, increasing toward +Y
 * (`Math.atan2(y, x)` convention) — pitch is the angle above/below the
 * horizontal XY plane, positive looking up.
 */

export type V3 = [number, number, number]

/** Default eye height above the ground plane (meters) — SketchUp's 5'6"
 * convention (design §4). Session-shared across all three tools; VCB-typed
 * changes persist for the rest of the session, not just the current tool. */
export const DEFAULT_EYE_HEIGHT_M = 1.68

/** Pitch clamp (degrees) for Look Around/Walk — design §4. Kept strictly
 * short of 90° so `forwardFromYawPitch` never produces an exactly-vertical
 * forward vector, which would make `headingFromForward` (yaw extraction)
 * degenerate on the NEXT gesture that reads this pose back. */
export const PITCH_CLAMP_DEG = 89.9

function toRad(deg: number): number {
  return (deg * Math.PI) / 180
}

/**
 * Extracts the horizontal heading (yaw, radians) from a `forward` direction
 * of any pitch — the "pre-click view heading" Position Camera's click
 * gesture preserves (design §4). Not required to be normalized or exactly
 * horizontal; only the XY components matter.
 *
 * A `forward` with a negligible horizontal component (looking within
 * `epsilon` of straight up/down — the zenith/nadir) has no meaningful
 * heading to preserve; returns `0` (arbitrary but deterministic) rather
 * than `NaN` from `atan2(0, 0)`.
 */
export function headingFromForward(forward: V3, epsilon = 1e-9): number {
  const [x, y] = forward
  if (Math.hypot(x, y) < epsilon) return 0
  return Math.atan2(y, x)
}

/**
 * Builds a unit forward vector from `yawRad` (compass angle, XY plane) and
 * `pitchRad` (angle above horizontal, clamped to ±`PITCH_CLAMP_DEG`) — the
 * inverse of `headingFromForward` at `pitchRad = 0`.
 */
export function forwardFromYawPitch(yawRad: number, pitchRad: number): V3 {
  const clampRad = toRad(PITCH_CLAMP_DEG)
  const pitch = Math.min(clampRad, Math.max(-clampRad, pitchRad))
  const cp = Math.cos(pitch)
  return [cp * Math.cos(yawRad), cp * Math.sin(yawRad), Math.sin(pitch)]
}

/** A resolved camera pose: eye position and a (not necessarily normalized,
 * callers should treat it as a look-AT direction, not a unit vector)
 * forward direction. */
export interface EyePose {
  eye: V3
  forward: V3
}

/**
 * Position Camera's CLICK gesture (design §4): eye placed `eyeHeight` above
 * `clickPoint`, looking horizontally (pitch 0) along the heading extracted
 * from `preClickForward` (the camera's forward direction just before the
 * click — whatever the previous tool/view left it at).
 */
export function positionCameraClick(clickPoint: V3, eyeHeight: number, preClickForward: V3): EyePose {
  const yaw = headingFromForward(preClickForward)
  return {
    eye: [clickPoint[0], clickPoint[1], clickPoint[2] + eyeHeight],
    forward: forwardFromYawPitch(yaw, 0),
  }
}

/**
 * Position Camera's DRAG gesture (design §4): eye at `eyeHeight` above the
 * press point, looking toward the release point. Unlike the click gesture,
 * the look direction is the true 3D direction to the release point (not
 * flattened to horizontal) — a drag onto a raised feature looks AT it, not
 * merely toward its ground shadow. Falls back to the pre-drag forward's
 * heading (horizontal) if press and release coincide (a degenerate,
 * effectively-zero-length drag).
 */
export function positionCameraDrag(pressPoint: V3, releasePoint: V3, eyeHeight: number, preClickForward: V3): EyePose {
  const eye: V3 = [pressPoint[0], pressPoint[1], pressPoint[2] + eyeHeight]
  const toRelease: V3 = [releasePoint[0] - eye[0], releasePoint[1] - eye[1], releasePoint[2] - eye[2]]
  const len = Math.hypot(toRelease[0], toRelease[1], toRelease[2])
  if (len < 1e-9) {
    return { eye, forward: forwardFromYawPitch(headingFromForward(preClickForward), 0) }
  }
  return { eye, forward: [toRelease[0] / len, toRelease[1] / len, toRelease[2] / len] }
}

/** Look Around's yaw/pitch after a drag of `dxPx`/`dyPx` screen pixels at
 * `radPerPixel` sensitivity (design §4): dragging right turns the view
 * right, dragging up (`dyPx < 0`) increases pitch (looks up) — the
 * conventional "drag the world" screen mapping every other orbit/look
 * gesture in this app uses (the world swings left under the cursor as the
 * view turns right). Pitch is clamped to ±`PITCH_CLAMP_DEG`; yaw is
 * unbounded (wraps naturally through `Math.cos`/`Math.sin` in
 * `forwardFromYawPitch`, so it is never itself normalized to ±π here).
 *
 * Yaw INCREASES counterclockwise as seen from above (this module's
 * `atan2(y, x)` heading convention), so `d(forward)/d(yaw) = -right`
 * (`horizontalBasis`) — turning the view toward its own right vector means
 * DECREASING yaw, not increasing it. A drag right must therefore SUBTRACT
 * from yaw, matching `walkDrag`'s identical fix below. */
export function lookAroundDrag(
  yawRad: number,
  pitchRad: number,
  dxPx: number,
  dyPx: number,
  radPerPixel: number,
): { yawRad: number; pitchRad: number } {
  const clampRad = toRad(PITCH_CLAMP_DEG)
  const newYaw = yawRad - dxPx * radPerPixel
  const newPitch = Math.min(clampRad, Math.max(-clampRad, pitchRad - dyPx * radPerPixel))
  return { yawRad: newYaw, pitchRad: newPitch }
}

/** One Walk drag's effect (design §4): normally vertical screen delta walks
 * forward/back along the HORIZONTAL forward direction and horizontal delta
 * turns (yaw); holding Shift swaps the mapping to strafe (horizontal delta)
 * and eye-height change (vertical delta) instead — SketchUp's own Walk
 * modifier. Exactly one of `{forwardDeltaM, strafeDeltaM}` and one of
 * `{yawDeltaRad, heightDeltaM}` is ever nonzero for a given call. */
export interface WalkDelta {
  yawDeltaRad: number
  forwardDeltaM: number
  strafeDeltaM: number
  heightDeltaM: number
}

/**
 * Computes one Walk drag's pose delta from screen-pixel deltas since the
 * press point (NOT since the last move — Walk's speed is proportional to
 * how far the cursor has traveled from where the drag started, like
 * SketchUp's own click-and-hold-to-walk feel, not a per-frame velocity).
 * `dxPx`/`dyPx` are both measured from the press point. Dragging UP
 * (`dyPx < 0`) walks forward (or raises the eye, under Shift); dragging
 * RIGHT (`dxPx > 0`) turns right (or strafes right, under Shift).
 *
 * The turn is a NEGATIVE yaw delta — same reasoning as `lookAroundDrag`'s
 * doc comment: this module's yaw increases counterclockwise from above, so
 * turning toward the view's own right vector subtracts from yaw.
 */
export function walkDrag(
  dxPx: number,
  dyPx: number,
  turnRadPerPixel: number,
  moveMPerPixel: number,
  shift: boolean,
): WalkDelta {
  if (shift) {
    return {
      yawDeltaRad: 0,
      forwardDeltaM: 0,
      strafeDeltaM: dxPx * moveMPerPixel,
      heightDeltaM: -dyPx * moveMPerPixel,
    }
  }
  return {
    yawDeltaRad: -dxPx * turnRadPerPixel,
    forwardDeltaM: -dyPx * moveMPerPixel,
    strafeDeltaM: 0,
    heightDeltaM: 0,
  }
}

/** The horizontal (Z=0-projected, unit) forward/right vectors for `yawRad` —
 * the basis Walk's forward/strafe deltas move the eye along. `right` is
 * `forward` rotated -90° in the XY plane (screen-right when facing
 * `forward` with +Z up), matching a standard right-handed walk basis. */
export function horizontalBasis(yawRad: number): { forward: V3; right: V3 } {
  const forward: V3 = [Math.cos(yawRad), Math.sin(yawRad), 0]
  const right: V3 = [Math.sin(yawRad), -Math.cos(yawRad), 0]
  return { forward, right }
}
