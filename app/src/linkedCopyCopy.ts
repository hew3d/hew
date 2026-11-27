/**
 * The words for linked copies. One place so every surface says the same
 * thing: a linked Object is an Object that happens to be linked, never
 * "definition" or "instance".
 */

import { t, plural } from '@lingui/core/macro'

/** Status line after Union or Intersect had to unlink placements to join them. */
export function unlinkedToJoin(count: number): string {
  return plural(count, { one: 'Unlinked # copy to join it', other: 'Unlinked # copies to join them' })
}

/** The undo label for a compound step that unlinked copies on the way. */
export function unlinkedLabel(label: string): string {
  return t({ comment: 'Undo label suffix; the first word is the action, e.g. "Union (unlinked)"', message: `${label} (unlinked)` })
}

export const cutOnlyThisOne = (): string => t`Unlinked this copy and cut it`

export function cutAllCopies(count: number): string {
  return plural(count, { one: 'Cut all # copy', other: 'Cut all # copies' })
}

export const onlyThisOne = (): string => t`Only this one`

/** "N linked" — beside an Object's type, and on the Outliner's count badge. */
export function linkedCountLabel(count: number): string {
  return t({ comment: 'How many linked placements share one shape; shown beside the type in Object Info and as the Outliner badge tooltip', message: `${count} linked` })
}

export const unlinkLabel = (): string => t`Unlink`
export const unlinkAllLabel = (): string => t`Unlink all`
export const makeComponentLabel = (): string => t`Make component`
export const selectAllCopiesLabel = (): string => t`Select all copies`
export const pasteIndependentLabel = (): string => t`Paste independent`
export const pasteLinkedLabel = (): string => t`Paste linked`
export const cutOneSolidAtATime = (): string => t`Cut with one solid at a time`
