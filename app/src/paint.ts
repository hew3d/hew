/**
 * nextPaint — resolve only after the browser has painted the current DOM.
 *
 * Used after a flushSync() whose result must be *visible* before a long
 * synchronous block (e.g. a main-thread WASM import) seizes the main thread.
 *
 * Why a DOUBLE requestAnimationFrame: rAF callbacks run at the *start* of a
 * frame, before that frame is painted.  Resolving the promise inside a single
 * rAF callback schedules the awaiting continuation as a microtask, and
 * microtasks drain before the browser reaches paint — so a synchronous block
 * started there still preempts the paint and the flushed DOM never shows up
 * until the freeze ends.  A second rAF callback cannot fire until the next
 * frame begins, which means the first frame — with the committed DOM — has
 * already been painted by the time it runs.  Resolving there guarantees the
 * paint has happened.
 */
export function nextPaint(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => resolve())
    })
  })
}
