/**
 * The attribute a part of Settings that is not a popup sets while Escape
 * belongs to it (a page inside a pane that Escape closes, say): mark it
 * with `data-settings-escape-child` and Settings stands down as for a
 * popup.
 */
export const SETTINGS_ESCAPE_CHILD = 'data-settings-escape-child'

/**
 * Whether a popup, menu or dialog other than the Settings dialog itself is
 * open, or a part of Settings marked `data-settings-escape-child`. Escape
 * then belongs to that child (it closes first, a second Escape closes
 * Settings), so Settings' capture-phase Escape handler stands down.
 */
export function childPopupOpen(settingsDialog: Element | null, root: ParentNode = document): boolean {
  const open = root.querySelectorAll(`[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"], dialog[open], [${SETTINGS_ESCAPE_CHILD}]`)
  for (const el of open) {
    if (el === settingsDialog) continue
    return true
  }
  return false
}
