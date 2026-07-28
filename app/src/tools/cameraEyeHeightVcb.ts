/**
 * cameraEyeHeightVcb — shared VCB (measurements-box) typed-entry logic for
 * eye height, common to `PositionCameraTool`/`LookAroundTool`/`WalkTool`
 * (docs/design/camera.md §4: eye height is VCB-editable in all three, and
 * the SAME session-shared value). Factored out once rather than tripled.
 *
 * Digit/period keys are captured unconditionally (there is no tool-switch
 * shortcut that is a bare digit); every other length-grammar key (unit
 * suffixes, quote marks, Backspace, Enter) is captured only once a digit has
 * already started the buffer — mirroring `Viewport.tsx`'s
 * `handleFovEntryKey` (the Zoom tool's own typed-FOV entry), which exists
 * precisely to avoid swallowing bare-letter tool shortcuts (`M` for Move,
 * `Spc` for Select, etc.) while idle. No three.js or DOM imports — fully
 * testable in Node/vitest.
 */

import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { parseLengthToMeters, typedReadout, type LengthFormat } from '../settings/units'

/** Whether `key` should be routed into the eye-height VCB buffer, given the
 * buffer's current contents `typed` (see the module doc for the
 * digit-first/letter-after-start rule). */
export function eyeHeightCapturesKey(typed: string, key: string): boolean {
  if (/^[0-9.]$/.test(key)) return true
  if (typed === '') return false
  return isLengthInputKey(key) || key === 'Enter'
}

/** Result of handling one keydown against the eye-height VCB buffer. */
export interface EyeHeightKeyResult {
  /** The buffer's new contents (`''` after a commit or a key this module
   * doesn't recognize). */
  typed: string
  /** The newly committed eye height (meters), or `null` if this keystroke
   * didn't commit one (anything but a valid `Enter`). Always `> 0` when
   * non-null — an invalid or non-positive typed value commits nothing and
   * simply clears the buffer, matching other VCB fields' "bad input is
   * silently discarded" convention. */
  committed: number | null
  /** Status-bar measurement readout for the buffer's new contents. */
  readout: string
}

/** Handles one keydown against the eye-height VCB. Callers should only call
 * this when `eyeHeightCapturesKey(typed, ev.key)` is true (or when routing
 * unconditionally from a dedicated eye-height entry point); an
 * unrecognized key is a no-op that returns `typed` unchanged. */
export function eyeHeightHandleKey(typed: string, key: string, format: LengthFormat): EyeHeightKeyResult {
  if (key === 'Enter') {
    const meters = parseLengthToMeters(typed)
    return { typed: '', committed: meters !== null && meters > 0 ? meters : null, readout: '' }
  }
  if (/^[0-9.]$/.test(key) || (typed !== '' && isLengthInputKey(key))) {
    const next = editLengthBuffer(typed, key, format)
    return { typed: next, committed: null, readout: typedReadout(next, format) }
  }
  return { typed, committed: null, readout: typedReadout(typed, format) }
}
