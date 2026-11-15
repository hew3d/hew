/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * toastActions — the button some toasts carry beside their message.
 *
 * A boolean refused because an operand has a face with no area says how to
 * clear it: the toast offers Model Health, listing the selected objects
 * (the operands) so "Remove degenerate faces" is one click away.
 */

import type { HealthScope } from './health/healthReport'

export interface ToastAction {
  label: string
  run: () => void
}

/** How long a toast with a button stays up, in milliseconds. */
export const ACTION_TOAST_MS = 8000

export function toastActionFor(
  code: string | undefined,
  openModelHealth: (view: { scope: HealthScope }) => void,
): ToastAction | undefined {
  if (code === 'DegenerateInput') {
    return { label: 'Open Model Health', run: () => openModelHealth({ scope: 'selection' }) }
  }
  // A fillet refused on an object with gaps: Model Health's Make Solid is
  // the way to close them. The picked object is not selected, so the whole
  // model is listed.
  if (code === 'BlendNeedsSolid') {
    return { label: 'Open Model Health', run: () => openModelHealth({ scope: 'document' }) }
  }
  return undefined
}
