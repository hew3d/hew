/**
 * Whether a popup, menu or dialog other than the Settings dialog itself is
 * open. Escape then belongs to that child (it closes first, a second Escape
 * closes Settings), so Settings' capture-phase Escape handler stands down.
 */
export function childPopupOpen(settingsDialog: Element | null, root: ParentNode = document): boolean {
  const open = root.querySelectorAll('[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"], dialog[open]')
  for (const el of open) {
    if (el === settingsDialog) continue
    return true
  }
  return false
}
