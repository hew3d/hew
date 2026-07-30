/**
 * Cross-browser click-count for `pointerdown`.
 *
 * `UIEvent.detail` carries the running click count (1, 2, 3…) on
 * MouseEvent-derived events, and the Viewport uses it to recognise the SECOND
 * pointerdown of a double-click — the phantom press that precedes the
 * `dblclick` event — so tools that finish on double-click (LineTool ending a
 * chain, PushPullTool repeating its last distance) don't also receive it as an
 * ordinary click.
 *
 * On Chromium that read is always 0. `PointerEvent` inherits `detail` from
 * `UIEvent`, and the Pointer Events spec leaves it at 0 for pointer events;
 * only the MouseEvent-derived `mousedown`/`click`/`dblclick` get real counts.
 * Measured directly in Chromium against a real double-click:
 *
 *     pointerdown detail=0 | mousedown detail=1 | click detail=1
 *     pointerdown detail=0 | mousedown detail=2 | click detail=2 | dblclick detail=2
 *
 * WebKit populates it, which is why this only ever showed up on Chromium
 * targets — the web app in Chrome/Edge, and the Windows desktop shell, whose
 * WebView2 is Chromium. `mousedown` cannot stand in: it fires AFTER
 * `pointerdown`, so its count arrives too late to decide whether to route the
 * press.
 *
 * This tracker reconstructs the count from press timing and position. It is
 * deliberately STRICTER than any browser's own double-click threshold, because
 * the two ways of being wrong are not symmetric:
 *
 * - Under-counting (we say 1, the browser says 2) leaves the phantom press
 *   routed — exactly today's Chromium behaviour, so never a regression.
 * - Over-counting (we say 2, the browser says 1) SWALLOWS A REAL CLICK: the
 *   press is suppressed and no `dblclick` ever arrives to stand in for it.
 *
 * So the window is tight enough that a press inside it is one the browser is
 * all but certain to pair up too, and callers combine this with the native
 * value (`Math.max(ev.detail, tracker.press(ev))`) so a browser that does
 * report a count keeps deciding for itself.
 */

/** Maximum gap between presses that can still form a multi-click, in ms.
 *  Chromium and WebKit both use 500 ms; the margin here is the deliberate
 *  strictness described above. */
export const MULTI_CLICK_MS = 400

/** Maximum movement between presses that can still form a multi-click, in CSS
 *  pixels. Comfortably inside the ~5 px the browsers themselves allow. */
export const MULTI_CLICK_SLOP_PX = 4

/** The fields this tracker reads — a structural subset of `PointerEvent`, so
 *  unit tests can drive it without synthesising real events. */
export interface MultiClickPress {
  timeStamp: number
  clientX: number
  clientY: number
  button: number
  pointerType?: string
}

export class MultiClickTracker {
  private lastTime = Number.NEGATIVE_INFINITY
  private lastX = 0
  private lastY = 0
  private lastButton = -1
  private lastPointerType: string | undefined = undefined
  private count = 0

  /**
   * Records a press and returns the click count it represents: 1 for a fresh
   * click, 2 for the second of a double-click, and so on (it keeps counting
   * past 2, matching `detail`). A press with a different button or pointer
   * type than the last one always restarts at 1 — a right-click between two
   * left-clicks breaks the sequence, as it does natively.
   */
  press(ev: MultiClickPress): number {
    const dt = ev.timeStamp - this.lastTime
    const dx = ev.clientX - this.lastX
    const dy = ev.clientY - this.lastY
    const continues =
      this.count > 0 &&
      ev.button === this.lastButton &&
      ev.pointerType === this.lastPointerType &&
      // `dt >= 0` guards a non-monotonic timestamp (a document.timeline reset,
      // or a synthesised event carrying a stale stamp): a negative gap is not
      // evidence of a fast second click, so it starts a fresh sequence.
      dt >= 0 &&
      dt <= MULTI_CLICK_MS &&
      dx * dx + dy * dy <= MULTI_CLICK_SLOP_PX * MULTI_CLICK_SLOP_PX

    this.count = continues ? this.count + 1 : 1
    this.lastTime = ev.timeStamp
    this.lastX = ev.clientX
    this.lastY = ev.clientY
    this.lastButton = ev.button
    this.lastPointerType = ev.pointerType
    return this.count
  }

  /**
   * Forgets the sequence, so the next press counts as 1. The Viewport calls
   * this when something happens between presses that should break the pairing
   * — a tool switch, or the window losing focus — for the same reason the
   * button/pointer-type checks exist: two clicks either side of an unrelated
   * event are not a double-click, however close together they land.
   */
  reset(): void {
    this.count = 0
    this.lastTime = Number.NEGATIVE_INFINITY
    this.lastButton = -1
    this.lastPointerType = undefined
  }
}
