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
 *  pixels. Comfortably inside the ~5 px the browsers themselves allow. This
 *  is the MOUSE-tuned value — a real fingertip's second tap lands nowhere
 *  near this precisely (shop-mode playtest finding 6: Kurt's double-tap
 *  zoom "works in tests" — synthetic taps land pixel-identical — but not on
 *  a real phone). See `touchSlopPx` below for the touch-specific value this
 *  tracker actually uses per-press. */
export const MULTI_CLICK_SLOP_PX = 4

/**
 * Same idea as `MULTI_CLICK_SLOP_PX`, but for touch input — a real
 * fingertip's second tap commonly lands 15-30px from the first even for a
 * deliberate double-tap on the same visual target (finger roll, screen
 * curvature, the touch target itself being bigger than a pixel). Picked
 * per-press from `MultiClickPress.pointerType` (`press`/`release` below),
 * so a mouse or pen press on the SAME tracker (e.g. the desktop editor run
 * on a touch-capable device — a rare but real path) keeps the tight
 * mouse-tuned slop untouched; only `pointerType === 'touch'` widens.
 */
export const MULTI_CLICK_TOUCH_SLOP_PX = 24

/** The fields this tracker reads — a structural subset of `PointerEvent`, so
 *  unit tests can drive it without synthesising real events. */
export interface MultiClickPress {
  timeStamp: number
  clientX: number
  clientY: number
  button: number
  pointerType?: string
}

/** `MULTI_CLICK_TOUCH_SLOP_PX` for a touch press, `MULTI_CLICK_SLOP_PX`
 *  (unchanged) for everything else — mouse, pen, or an event that never
 *  reports a `pointerType` at all (e.g. a plain synthesised `MouseEvent`,
 *  `detail`-only). */
function slopPxFor(pointerType: string | undefined): number {
  return pointerType === 'touch' ? MULTI_CLICK_TOUCH_SLOP_PX : MULTI_CLICK_SLOP_PX
}

export class MultiClickTracker {
  /** Where and when the last COMPLETED click landed — a click being a press
   *  that was released without travelling (see `release`). `null` until one
   *  completes, and cleared whenever a press turns out to be a drag. */
  private lastClick: { time: number; x: number; y: number } | null = null
  private lastButton = -1
  private lastPointerType: string | undefined = undefined
  private count = 0
  /** The press currently in flight, so `release` can measure how far it
   *  travelled. */
  private pending: { x: number; y: number } | null = null

  /**
   * Records a press and returns the click count it represents: 1 for a fresh
   * click, 2 for the second of a double-click, and so on (it keeps counting
   * past 2, matching `detail`). A press with a different button or pointer
   * type than the last one always restarts at 1 — a right-click between two
   * left-clicks breaks the sequence, as it does natively.
   */
  press(ev: MultiClickPress): number {
    const prev = this.lastClick
    const dt = prev === null ? Number.POSITIVE_INFINITY : ev.timeStamp - prev.time
    const dx = prev === null ? 0 : ev.clientX - prev.x
    const dy = prev === null ? 0 : ev.clientY - prev.y
    const slopPx = slopPxFor(ev.pointerType)
    const continues =
      prev !== null &&
      this.count > 0 &&
      ev.button === this.lastButton &&
      ev.pointerType === this.lastPointerType &&
      // `dt >= 0` guards a non-monotonic timestamp (a document.timeline reset,
      // or a synthesised event carrying a stale stamp): a negative gap is not
      // evidence of a fast second click, so it starts a fresh sequence.
      dt >= 0 &&
      dt <= MULTI_CLICK_MS &&
      dx * dx + dy * dy <= slopPx * slopPx

    this.count = continues ? this.count + 1 : 1
    this.lastButton = ev.button
    this.lastPointerType = ev.pointerType
    this.pending = { x: ev.clientX, y: ev.clientY }
    return this.count
  }

  /**
   * Closes the press opened by `press`, deciding whether it was a CLICK (and
   * so can anchor a double-click) or a DRAG (which cannot).
   *
   * This distinction is the whole reason `release` exists. Browsers pair a
   * double-click from consecutive CLICKS, comparing where each click was
   * RELEASED; a press that travels before release is a drag and is never one
   * half of a pair. Measuring only press-to-press distance misses that
   * completely: a drag from P to Q followed within the window by a fresh press
   * back at P looks like two presses at the same spot, so the second would be
   * suppressed while the browser — seeing releases at Q and then wherever the
   * second ends — fires no `dblclick` to replace it. That swallows a real
   * gesture outright, which for a tool like Push/Pull (armed only from its
   * idle stage, on pointerdown, with no later fallback) silently drops the
   * whole interaction.
   *
   * A press that travelled therefore ends the sequence, and the anchor for the
   * next comparison is the RELEASE position, matching what the browser pairs
   * on rather than approximating it with the press position.
   */
  release(ev: MultiClickPress): void {
    const down = this.pending
    this.pending = null
    if (down === null) return
    const dx = ev.clientX - down.x
    const dy = ev.clientY - down.y
    const slopPx = slopPxFor(ev.pointerType)
    if (dx * dx + dy * dy > slopPx * slopPx) {
      // A drag. It anchors nothing, and it breaks any run it landed in.
      this.lastClick = null
      this.count = 0
      return
    }
    this.lastClick = { time: ev.timeStamp, x: ev.clientX, y: ev.clientY }
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
    this.lastClick = null
    this.pending = null
    this.lastButton = -1
    this.lastPointerType = undefined
  }
}
