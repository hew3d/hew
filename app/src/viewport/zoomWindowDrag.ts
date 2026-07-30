/**
 * zoomWindowDrag — pure arm/drag decision logic for Camera ▸ Zoom Window
 * (docs/design/camera.md §3): a one-shot screen rectangle that reframes the
 * camera on release, reusing the marquee's rubber-band visuals.
 *
 * Mirrors dragMove.ts's split: pure data in, pure data out, so the state
 * machine that decides "has this press become a drag yet, and what
 * rectangle does it describe" is unit-testable without three.js, wasm, or a
 * real canvas. Viewport.tsx owns the DOM/pointer-capture side effects and
 * the actual camera reframe (`applyZoomWindow`); this module owns none of
 * that.
 */

import { normalizedRect, type MarqueeRect } from './marquee'

/** Pixels of pointer travel that arm the rectangle — matches the Select
 * tool's marquee threshold so "click" means the same thing everywhere. */
export const ZOOM_WINDOW_DRAG_THRESHOLD_PX = 5

export interface ZoomWindowDragState {
  readonly startX: number
  readonly startY: number
  readonly active: boolean
}

/** Arm a fresh drag at the press position. Not yet "active" — a plain
 * click (no travel past the threshold) stays a one-shot no-op release. */
export function beginZoomWindowDrag(x: number, y: number): ZoomWindowDragState {
  return { startX: x, startY: y, active: false }
}

/**
 * Advance an armed drag to a new pointer position.
 *
 * `buttonsDown` is the live primary-button bit (`ev.buttons & 1`) — false
 * means the release happened outside our listeners (focus loss), which the
 * caller treats exactly like an Escape abort: drop the state, no reframe.
 *
 * Returns the next state (or `null` if the drag should be abandoned) and
 * the rectangle to draw, which is `null` until the drag first crosses the
 * arming threshold — a single call is enough to arm it; no priming cycle
 * is needed (this is the regression surface for "first drag ignored,
 * second works": the decision here depends on nothing but this call's own
 * arguments, never on how many times the function has previously run).
 */
export function updateZoomWindowDrag(
  state: ZoomWindowDragState,
  x: number,
  y: number,
  buttonsDown: boolean,
): { state: ZoomWindowDragState | null; rect: MarqueeRect | null } {
  if (!buttonsDown) return { state: null, rect: null }
  const active = state.active
    || Math.hypot(x - state.startX, y - state.startY) >= ZOOM_WINDOW_DRAG_THRESHOLD_PX
  const next: ZoomWindowDragState = active === state.active ? state : { ...state, active }
  return { state: next, rect: active ? normalizedRect(state.startX, state.startY, x, y) : null }
}

/** The rectangle to commit on release, or `null` if the drag never crossed
 * the arming threshold (a plain click — the one-shot still reverts to
 * Select, it just doesn't reframe). */
export function finishZoomWindowDrag(
  state: ZoomWindowDragState,
  x: number,
  y: number,
): MarqueeRect | null {
  return state.active ? normalizedRect(state.startX, state.startY, x, y) : null
}
