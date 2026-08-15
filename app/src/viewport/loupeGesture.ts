/**
 * loupeGesture — the pure press/hold/slop/disengage state machine behind
 * the Tape Measure loupe (shop-mode round-3 playtest finding 4): a
 * fingertip covers far more screen than the aperture a thin part or a
 * cluster of nearby endpoints needs, so a plain tap under `readOnly` can't
 * reliably land on one. Holding still for `LOUPE_HOLD_MS` with less than
 * `LOUPE_SLOP_PX` of drift engages a magnified, camera-frozen probe instead
 * of committing wherever the raw touch-down happened to land.
 *
 * Pure and DOM-free by design — no timer, no pointer, no wasm — so the
 * timer/slop/disengage machinery is unit-testable with plain numbers, the
 * same "pure reduction, host owns the side effects" shape as this file's
 * neighbors (`marquee.ts`, `dragMove.ts`, `sheetDetents.ts`). The caller
 * supplies wall-clock time itself (`performance.now()`, or a fake clock in
 * tests) rather than this module starting its own `setTimeout` — a real
 * timer still has to live in the host (nothing else can wake a perfectly
 * STILL hold once no further pointer event is coming), but the host's
 * timer callback is just one more caller of `loupeTick`, not a second
 * source of truth about elapsed time.
 *
 * States:
 * - `idle` — no gesture.
 * - `armed` — pressed, waiting to see whether this becomes a hold (engage),
 *   a quick tap (release before either threshold), or a real drag
 *   (`rejected`).
 * - `engaged` — held past `LOUPE_HOLD_MS` without drifting past
 *   `LOUPE_SLOP_PX`. Tracks the live finger position for the magnified
 *   probe; a release from here commits.
 * - `rejected` — the slop radius broke before the hold could engage: a
 *   drag/orbit, not a hold. Terminal until the next press — `loupeMove`/
 *   `loupeTick` are no-ops here so the host doesn't need to keep checking
 *   once it has reacted to the transition once.
 */

/** Hold duration (ms) before an armed press engages the loupe. */
export const LOUPE_HOLD_MS = 300

/** Movement radius (px) an armed press may drift within before the hold is
 *  disqualified (`rejected`) — the same "this became a drag" signal every
 *  other armed gesture in `Viewport.tsx` uses (`MARQUEE_DRAG_THRESHOLD_PX`,
 *  `dragMove.ts`'s own threshold), reused here rather than a third
 *  independently-tuned constant. */
export const LOUPE_SLOP_PX = 10

export type LoupeState =
  | { phase: 'idle' }
  | { phase: 'armed'; startX: number; startY: number; startTime: number }
  | { phase: 'engaged'; x: number; y: number }
  | { phase: 'rejected' }

export const LOUPE_IDLE: LoupeState = { phase: 'idle' }

/** A press (pointerdown) — always starts fresh in `armed`, discarding
 *  whatever state came before (a caller that presses again mid-gesture has
 *  a bug elsewhere; this module doesn't try to guess intent from it). */
export function loupePress(x: number, y: number, startTime: number): LoupeState {
  return { phase: 'armed', startX: x, startY: y, startTime }
}

/**
 * A pointermove during the gesture.
 *
 * `armed`: past `LOUPE_SLOP_PX` of drift from the press point disqualifies
 * the hold (`rejected`) — this is a drag, and the host's job from here is
 * to let it behave like an ordinary one (see `Viewport.tsx`'s own
 * integration doc for what "ordinary" means for Tape Measure specifically).
 * Below the slop radius, `armed` doesn't move at all — the loupe's whole
 * point is fine positioning from a STILL hold, so drift under the
 * threshold is deliberately absorbed rather than tracked.
 *
 * `engaged`: the slop radius no longer applies (holding still was already
 * proven) — every move updates the tracked probe position outright, which
 * is what lets the user slide their finger to fine-tune after engaging.
 *
 * `idle`/`rejected`: no-op.
 */
export function loupeMove(state: LoupeState, x: number, y: number): LoupeState {
  if (state.phase === 'armed') {
    const dist = Math.hypot(x - state.startX, y - state.startY)
    return dist >= LOUPE_SLOP_PX ? { phase: 'rejected' } : state
  }
  if (state.phase === 'engaged') return { phase: 'engaged', x, y }
  return state
}

/**
 * A wall-clock check — from the host's own `setTimeout(LOUPE_HOLD_MS)`
 * firing (the only way to detect a perfectly still hold, which produces no
 * further pointer events of its own to re-check against), or from any
 * other event the host wants to re-derive elapsed time at. Engages an
 * `armed` press once `now - startTime >= LOUPE_HOLD_MS`; a no-op in every
 * other phase, `engaged` included (a time check has nothing further to
 * decide there).
 */
export function loupeTick(state: LoupeState, now: number): LoupeState {
  if (state.phase === 'armed' && now - state.startTime >= LOUPE_HOLD_MS) {
    return { phase: 'engaged', x: state.startX, y: state.startY }
  }
  return state
}

/**
 * A release (pointerup). Reports whether the gesture had engaged — the
 * host's cue to commit the current probe as the tape point — alongside the
 * reset state, which is unconditionally `idle`: a release always ends the
 * gesture regardless of phase (an `armed` release is a plain quick tap, a
 * `rejected` release has nothing left to do — its commit already happened
 * at the moment of rejection, see the module doc — and an `engaged`
 * release is the fine-positioned commit this whole gesture exists for).
 */
export function loupeRelease(state: LoupeState): { committed: boolean } {
  return { committed: state.phase === 'engaged' }
}

/** A cancel (pointercancel, or any other forced abort — window blur, a
 *  tool switch mid-gesture). Always discards the gesture with NO commit,
 *  regardless of phase — the one asymmetry with `loupeRelease`, which
 *  commits from `engaged`. */
export function loupeCancel(): LoupeState {
  return LOUPE_IDLE
}
