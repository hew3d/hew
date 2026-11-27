/* oxlint-disable lingui/no-unlocalized-strings -- style values. */
import type { CSSProperties } from 'react'

/** The Settings panes' push button, the same look as the Folders pane's. */
export function buttonStyle(disabled = false, primary = false): CSSProperties {
  return {
    padding: '4px 12px',
    fontSize: '13px',
    fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
    background: primary && !disabled ? 'var(--accent-strong, #3b6ae8)' : 'var(--surface-input, #2a2a2a)',
    color: disabled ? 'var(--text-faint, #666)' : primary ? '#fff' : 'var(--text-primary, #eee)',
    border: '1px solid var(--border-strong, #444)',
    borderRadius: '6px',
    cursor: disabled ? 'default' : 'pointer',
    flexShrink: 0,
  }
}
