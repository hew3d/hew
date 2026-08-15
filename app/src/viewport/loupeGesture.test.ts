import { describe, it, expect } from 'vitest'
import {
  LOUPE_HOLD_MS,
  LOUPE_SLOP_PX,
  LOUPE_IDLE,
  loupePress,
  loupeMove,
  loupeTick,
  loupeRelease,
  loupeCancel,
  type LoupeState,
} from './loupeGesture'

describe('loupeGesture (timer)', () => {
  it('a tick before LOUPE_HOLD_MS elapses stays armed', () => {
    const armed = loupePress(100, 100, 1000)
    expect(loupeTick(armed, 1000 + LOUPE_HOLD_MS - 1)).toEqual(armed)
  })

  it('a tick at exactly LOUPE_HOLD_MS engages, at the press position', () => {
    const armed = loupePress(100, 100, 1000)
    expect(loupeTick(armed, 1000 + LOUPE_HOLD_MS)).toEqual({ phase: 'engaged', x: 100, y: 100 })
  })

  it('a tick well past LOUPE_HOLD_MS also engages (a late-firing timer)', () => {
    const armed = loupePress(100, 100, 1000)
    expect(loupeTick(armed, 1000 + LOUPE_HOLD_MS + 5000)).toEqual({ phase: 'engaged', x: 100, y: 100 })
  })

  it('a tick is a no-op once idle, engaged, or rejected', () => {
    expect(loupeTick(LOUPE_IDLE, 999999)).toEqual(LOUPE_IDLE)
    const engaged: LoupeState = { phase: 'engaged', x: 5, y: 6 }
    expect(loupeTick(engaged, 999999)).toEqual(engaged)
    const rejected: LoupeState = { phase: 'rejected' }
    expect(loupeTick(rejected, 999999)).toBe(rejected)
  })
})

describe('loupeGesture (slop)', () => {
  it('a sub-threshold wiggle while armed stays armed, unchanged', () => {
    const armed = loupePress(100, 100, 0)
    expect(loupeMove(armed, 100 + LOUPE_SLOP_PX - 1, 100)).toBe(armed)
    expect(loupeMove(armed, 100, 100 - (LOUPE_SLOP_PX - 1))).toBe(armed)
  })

  it('crossing the slop radius while armed rejects the hold', () => {
    const armed = loupePress(100, 100, 0)
    expect(loupeMove(armed, 100 + LOUPE_SLOP_PX, 100)).toEqual({ phase: 'rejected' })
    // Diagonal distance counts, not per-axis deltas: 8px on EACH axis is
    // individually under the 10px slop, but their combined distance (~11.3)
    // is not.
    expect(loupeMove(loupePress(0, 0, 0), 8, 8)).toEqual({ phase: 'rejected' })
  })

  it('once engaged, the slop radius no longer applies — every move tracks', () => {
    const engaged: LoupeState = { phase: 'engaged', x: 50, y: 50 }
    expect(loupeMove(engaged, 500, 500)).toEqual({ phase: 'engaged', x: 500, y: 500 })
  })

  it('a move is a no-op once idle or rejected', () => {
    expect(loupeMove(LOUPE_IDLE, 1, 1)).toBe(LOUPE_IDLE)
    const rejected: LoupeState = { phase: 'rejected' }
    expect(loupeMove(rejected, 1, 1)).toBe(rejected)
  })
})

describe('loupeGesture (disengage)', () => {
  it('release from idle or armed (a plain quick tap) commits nothing', () => {
    expect(loupeRelease(LOUPE_IDLE)).toEqual({ committed: false })
    expect(loupeRelease(loupePress(0, 0, 0))).toEqual({ committed: false })
  })

  it('release from rejected (a real drag) commits nothing — that gesture already resolved itself elsewhere', () => {
    expect(loupeRelease({ phase: 'rejected' })).toEqual({ committed: false })
  })

  it('release from engaged commits the fine-positioned probe', () => {
    expect(loupeRelease({ phase: 'engaged', x: 1, y: 2 })).toEqual({ committed: true })
  })

  it('cancel always discards with no commit, from any phase', () => {
    expect(loupeCancel()).toEqual(LOUPE_IDLE)
  })

  it('a full press → hold → engage → release round trip', () => {
    let state = loupePress(10, 10, 0)
    state = loupeTick(state, LOUPE_HOLD_MS - 1)
    expect(state.phase).toBe('armed')
    state = loupeMove(state, 12, 11) // under slop, no-op
    expect(state.phase).toBe('armed')
    state = loupeTick(state, LOUPE_HOLD_MS)
    expect(state).toEqual({ phase: 'engaged', x: 10, y: 10 })
    state = loupeMove(state, 40, 30) // fine-position after engaging
    expect(state).toEqual({ phase: 'engaged', x: 40, y: 30 })
    expect(loupeRelease(state)).toEqual({ committed: true })
  })

  it('a full press → drag-past-slop → release never engages or commits', () => {
    let state = loupePress(10, 10, 0)
    state = loupeMove(state, 10 + LOUPE_SLOP_PX, 10)
    expect(state).toEqual({ phase: 'rejected' })
    // A late timer firing after rejection must not resurrect the hold.
    state = loupeTick(state, LOUPE_HOLD_MS + 100)
    expect(state).toEqual({ phase: 'rejected' })
    expect(loupeRelease(state)).toEqual({ committed: false })
  })

  it('a quick tap (release before the timer ever ticks) commits nothing — the host commits its OWN plain-tap outcome, not this module\'s', () => {
    const state = loupePress(10, 10, 0)
    expect(loupeRelease(state)).toEqual({ committed: false })
  })
})
