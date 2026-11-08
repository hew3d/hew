/**
 * The copy-state cycle shared by Move and Rotate.
 *
 * Tapping the copy modifier cycles plain Move, then Copy (the preferred
 * kind), then the other kind of copy, then back to Move. With "Copies are
 * linked" on, the first copy is linked and the alternate is independent;
 * with it off the roles swap. Array entry (xN, /N) follows the copy it
 * continues, so the chosen kind rides along.
 */

export type CopyState = 'off' | 'primary' | 'alternate'

/** Called when the copy state changes; `linked` is what a commit would make. */
export type OnCopyModeChange = (state: CopyState, linked: boolean) => void

export function nextCopyState(s: CopyState): CopyState {
  if (s === 'off') return 'primary'
  if (s === 'primary') return 'alternate'
  return 'off'
}

/** Whether a copy made under `s` is linked, given the "Copies are linked" preference. */
export function linkedFor(s: CopyState, preferLinked: boolean): boolean {
  if (s === 'off') return false
  return s === 'primary' ? preferLinked : !preferLinked
}

/** The cursor badge for `s`: none, a bare plus for the preferred kind, or the other kind spelled out. */
export function copyBadgeText(s: CopyState, preferLinked: boolean): '' | '+' | '+ independent' | '+ linked' {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- cursor badge ids, drawn as glyphs by toolIcons.cursorFor
  if (s === 'off') return ''
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- cursor badge ids
  if (s === 'primary') return '+'
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- cursor badge ids
  return preferLinked ? '+ independent' : '+ linked'
}
