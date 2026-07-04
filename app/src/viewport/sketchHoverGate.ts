/**
 * sketchHoverGate — pure throttle + edge-detect for the "hovering an
 * extrudable sketch region" signal ("sketches are first-class
 * interactable" contextual-dock half).
 *
 * Viewport.tsx's pointer-move handler lives inside one giant mount effect
 * with no dedicated component-test harness for raw pointer wiring (see the
 * hover-dock task's design note: Vite Fast Refresh strands `[]`-effect
 * listeners, a recurring gotcha in this codebase). So the two behaviors that
 * actually need coverage — throttling the expensive wasm ray-cast, and only
 * notifying the caller on a true/false TRANSITION rather than on every poll
 * — are pulled out into this standalone, side-effect-free module. Viewport
 * owns the wasm call and the pause conditions (selection non-empty, camera
 * nav, pointer button down); this module just does the bookkeeping.
 *
 * No React/DOM/wasm import — plain data in, plain data out.
 */

/** Minimum time between polls of the (relatively expensive) wasm ray pick. */
export const HOVER_POLL_THROTTLE_MS = 100

export class SketchHoverGate {
  private lastPollAt = -Infinity
  private lastEmitted = false

  /**
   * True when at least `HOVER_POLL_THROTTLE_MS` has elapsed since the last
   * poll — the caller should run the ray pick now and feed its result to
   * `update()`. Has the side effect of resetting the throttle clock to
   * `nowMs` whenever it returns true (so callers don't need a separate
   * "mark checked" step).
   */
  shouldPoll(nowMs: number): boolean {
    if (nowMs - this.lastPollAt < HOVER_POLL_THROTTLE_MS) return false
    this.lastPollAt = nowMs
    return true
  }

  /**
   * Feed the latest raw hover boolean (from a poll this tick). Returns the
   * value to emit to the caller's callback, or `null` if it's unchanged
   * since the last emit — so the caller only re-renders React on an actual
   * transition, never on every poll.
   */
  update(hovering: boolean): boolean | null {
    if (hovering === this.lastEmitted) return null
    this.lastEmitted = hovering
    return hovering
  }

  /**
   * Call instead of polling whenever hover-picking is PAUSED this tick
   * (explicit selection took over, a camera drag or tool drag is in
   * flight, ...). Forces the state back to "not hovering" — emitting a
   * false transition if it was previously true — and resets the throttle
   * clock so the next resumed poll isn't held back by a stale window.
   * Returns the value to emit, or `null` if it was already false.
   */
  pause(): boolean | null {
    this.lastPollAt = -Infinity
    return this.update(false)
  }

  /**
   * Clear the throttle clock without touching the last-emitted state, so the
   * very next `shouldPoll`/manual poll runs immediately instead of waiting out
   * a stale window. Unlike `pause()`, this does NOT force a false emission —
   * use it when the underlying document changed (undo/redo/delete/tool
   * commit) and the caller is about to re-poll with the real cursor position
   * right away, so the *current* truth should win, not an assumed "not
   * hovering."
   */
  reset(): void {
    this.lastPollAt = -Infinity
  }
}
