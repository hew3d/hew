/**
 * fovDrag — pure arm/drag decision logic for Shift+Zoom fov adjustment
 * (docs/design/camera-playtest2.md §3): while the Zoom camera mode is
 * active in perspective projection, a left-drag begun with Shift held
 * adjusts fov instead of dollying — the eye does not move. Shift+wheel
 * does the same thing in smaller steps.
 *
 * Mirrors zoomWindowDrag.ts's split: pure data in, pure data out, so the
 * multiplicative law and the fixed-at-press mode decision are
 * unit-testable without three.js, wasm, or a real canvas. Viewport.tsx
 * owns the DOM side effects: the `controls.mouseButtons.LEFT` pre-emption
 * (so OrbitControls' native dolly never sees the gesture), pointer
 * capture, the cursor swap, and restoring `MOUSE.DOLLY` on every exit path
 * (release, Escape, focus loss, pointer-capture loss, a tool switch away
 * from Zoom).
 *
 * MODE FIXED AT PRESS — a deliberate SketchUp deviation, not an oversight.
 * Whether a drag adjusts fov or dollies is decided ONCE, from the
 * pointerdown event's `shiftKey`, and never re-evaluated for the rest of
 * that gesture: a drag begun without Shift stays a dolly even if Shift is
 * pressed mid-drag, and a drag begun WITH Shift stays a fov adjustment even
 * if Shift is released mid-drag. This branch's own history is the reason —
 * every mid-drag modifier change in the camera work has produced a
 * stale-anchor jump (Walk's Shift-mid-drag bug and its sticky aftermath),
 * and OrbitControls' native `MOUSE.DOLLY` specifically cannot adopt or
 * release an in-flight drag: it decides its internal state machine once,
 * at its OWN pointerdown, from `mouseButtons.LEFT` at that instant, and
 * never re-reads it until the next press. Handing a live gesture between
 * it and our own handler is exactly the class of bug that cost two fix
 * rounds last time — fixed-at-press sidesteps it entirely by never
 * attempting a handoff.
 *
 * SIGN — established against THIS build's real dolly drag, not assumed
 * (see the design doc's ground-truth warning: a plausible-looking sign is
 * worth nothing, and this branch already shipped an inverted-yaw bug that
 * analysis alone didn't catch). Verified two ways: (1) three.js
 * OrbitControls' own `_handleMouseMoveDolly` — `dollyDelta.y < 0` (dragged
 * UP, screen Y decreasing) calls `_dollyIn` (camera moves closer); (2) a
 * real Playwright drive of this build's Zoom tool — dragging up 150px
 * measurably shrank camera-to-target distance. "Up = zoom in" is the
 * existing dolly's own convention; this drag matches it: up (decreasing Y)
 * NARROWS fov, down WIDENS it.
 */
import { MAX_FOV_DEG, MIN_FOV_DEG } from './cameraRig'

/** Pixels of drag travel that multiply fov by e — chosen so a 400px drag
 * (design doc: "roughly a full usable range" on a typical small viewport)
 * is about a 3x fov swing: 400 / ln(3). A multiplicative (not additive) law
 * in fov space, per the design doc — a constant degrees-per-pixel law reads
 * badly at the narrow end (a 1px move would be as visible at fov=2° as at
 * fov=90°). */
export const FOV_DRAG_K = 400 / Math.log(3)

/** Shift+wheel's law is the same shape, far less sensitive per unit of
 * `deltaY` — a wheel notch is a small nudge, not a drag-sized swing. */
export const FOV_WHEEL_K = FOV_DRAG_K * 10

function clampFov(fovDeg: number): number {
  return Math.min(MAX_FOV_DEG, Math.max(MIN_FOV_DEG, fovDeg))
}

export interface FovDragState {
  readonly pressY: number
  readonly baseFov: number
}

/** Arm a fov drag at the press position, pinning the fov it started from
 * (`rig.perspective.fov` at pointerdown) as the multiplicative base — every
 * subsequent tick scales from THIS fixed value, not the live fov, so the
 * result is a pure function of total travel from the press rather than of
 * frame timing or tick count. */
export function beginFovDrag(pressY: number, baseFov: number): FovDragState {
  return { pressY, baseFov }
}

/** fov at pointer position `y` during an armed drag, clamped to
 * [`MIN_FOV_DEG`, `MAX_FOV_DEG`] — the same bounds `CameraRig.setFov`
 * enforces (see that clamp's own doc for why it lives there and not here). */
export function fovDragValue(state: FovDragState, y: number): number {
  return clampFov(state.baseFov * Math.exp((y - state.pressY) / FOV_DRAG_K))
}

/** fov after one Shift+wheel tick — same multiplicative law and sign as
 * the drag (`deltaY` straight from the `WheelEvent`: positive/down = widen,
 * negative/up = narrow), through the much larger `FOV_WHEEL_K`. */
export function fovAfterWheel(baseFov: number, deltaY: number): number {
  return clampFov(baseFov * Math.exp(deltaY / FOV_WHEEL_K))
}

/**
 * The fixed-at-press mode decision itself (module doc above) — a single
 * named choke point so "the press decides, and only the press" is
 * expressed as one pure mapping the caller reads ONCE at pointerdown and
 * stores, rather than re-derived ad hoc (and potentially re-evaluated by
 * mistake) at every subsequent event. Viewport.tsx's own wiring is what
 * proves the "only the press" half of the contract (see the e2e specs);
 * this function only proves the mapping itself.
 */
export function decideFovDragMode(shiftHeldAtPress: boolean): 'fov' | 'dolly' {
  return shiftHeldAtPress ? 'fov' : 'dolly'
}
