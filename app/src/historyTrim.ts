/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * historyTrim — the status-bar note for the undo-memory budget.
 *
 * On a dense scan every step can hold a hundred megabytes or more, so the
 * kernel drops the oldest undo steps once the history passes its budget
 * (wasm `Scene.take_history_trim`). The user should hear about it once, in
 * plain words, without a dialog: a status-bar note that fades after a while.
 */

/** How long the note stays in the status bar, in milliseconds. */
export const HISTORY_TRIM_NOTE_MS = 12000

/** The note's text for a trim that left `kept` undo steps. */
export function historyTrimNote(kept: number): string {
  return `Undo history trimmed to save memory: ${kept} ${kept === 1 ? 'step' : 'steps'} kept`
}

/** The tooltip that explains the note. */
export const HISTORY_TRIM_TOOLTIP =
  'This model is large, so the oldest undo steps were let go to keep memory free. The most recent steps can still be undone.'

interface TrimSource {
  /** Optional so the app's test doubles of the scene need not carry it. */
  take_history_trim?(): { kept(): number; free(): void } | undefined
}

/** Collects the trim the last edit made, if any: the kept count, or null. */
export function takeHistoryTrim(scene: TrimSource): number | null {
  const trim = scene.take_history_trim?.()
  if (trim === undefined) return null
  const kept = trim.kept()
  trim.free()
  return kept
}
