/**
 * Merge the high-level kernel command stream (from Rust `Scene::take_recording`)
 * with the low-level `input` array (from {@link module:inputRecorder}) into one
 * session artifact — the "sibling array in the same artifact" of RECORDING_FORMAT
 *. The result is what M17's Report-Bug bundle ships and what 's harness
 * can hand back.
 *
 * ## Precision: never JSON.parse the high-level stream
 *
 * The high-level JSON carries `golden_hash` (a u64 `state_hash`) and slotmap
 * handles in `calls`, any of which can exceed `Number.MAX_SAFE_INTEGER`. Parsing
 * then re-stringifying would silently corrupt them (the runner hit exactly
 * this). So we treat the high-level JSON as **opaque text** and splice the
 * `input` array in by string surgery, never touching its numbers. The `input`
 * array is TS-originated (coords/timestamps/seq/fov — all safe doubles), so
 * stringifying *it* is fine.
 */

import type { InputEvent } from './inputRecorder'

/**
 * Splice `input` into the high-level recording JSON as a top-level `"input"`
 * array, without parsing the high-level numbers. With no events, returns the
 * high-level JSON unchanged (a kernel-only recording stays byte-identical to the
 * Rust output, so existing high-level fixtures are unaffected).
 *
 * `highLevelJson` must be a JSON object literal (serde's `Scene::take_recording`
 * output: `{"version":2,"calls":[…],"golden_hash":N}`); it ends in `}`.
 */
export function buildSessionRecording(
  highLevelJson: string,
  input: InputEvent[],
): string {
  if (input.length === 0) return highLevelJson
  const inputJson = JSON.stringify(input)
  // Insert before the final closing brace of the top-level object. serde emits
  // no trailing whitespace, but tolerate some defensively.
  const m = highLevelJson.match(/\}\s*$/)
  if (!m) {
    throw new Error(
      'buildSessionRecording: high-level recording is not a JSON object',
    )
  }
  const cut = highLevelJson.length - m[0].length
  return `${highLevelJson.slice(0, cut)},"input":${inputJson}}`
}

/**
 * Read just the `input` array back out of a session artifact, for the low-level
 * replay driver and tests. Safe to JSON.parse: `input` holds only TS
 * doubles, never the u64 `golden_hash`/handles. Returns `[]` if absent.
 *
 * (To read the high-level `golden_hash` precisely, extract it as text → BigInt —
 * do **not** rely on JSON.parse — per the runner.)
 */
export function extractInput(sessionJson: string): InputEvent[] {
  const parsed = JSON.parse(sessionJson) as { input?: InputEvent[] }
  return parsed.input ?? []
}
