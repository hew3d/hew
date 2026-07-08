/**
 * SettingsForm — shared macOS-HIG form-layout primitives for the Settings
 * panes (SettingsWindow.tsx).
 *
 * Implements the classic macOS preferences form grid: a right-aligned labels
 * column and a left-aligned controls column, 13px system font, hairline
 * separators between groups, and faint explanatory notes under controls.
 * All colors come from theme tokens (app/src/theme/tokens.css) so the same
 * markup is correct in both light and dark themes.
 *
 * `SettingsRow` renders its two cells as siblings (via a fragment) so both
 * land directly in `SettingsForm`'s grid — every row shares one label-column
 * width. The row label is a real `<label htmlFor>` when the control id is
 * given, and a plain `<span>` otherwise (e.g. when the control cell contains
 * its own `<label>`-wrapped checkbox — nesting labels is invalid HTML).
 */

import type { CSSProperties, ReactNode } from 'react'

const formStyle: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'minmax(96px, max-content) 1fr',
  columnGap: '12px',
  rowGap: '12px',
  alignItems: 'center',
  fontSize: '13px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
}

const rowLabelStyle: CSSProperties = {
  justifySelf: 'end',
  textAlign: 'right',
  color: 'var(--text-secondary, #ddd)',
}

const controlCellStyle: CSSProperties = {
  justifySelf: 'start',
  minWidth: 0,
}

/** The two-column form grid. Children are rows/separators/notes. */
export function SettingsForm({ children }: { children: ReactNode }) {
  return <div style={formStyle}>{children}</div>
}

/**
 * One label + control row. Pass `htmlFor` (the control's id) to make the
 * label a real `<label>`; omit it when the control carries its own label.
 * `alignTop` pins the label to the first line of a tall control cell.
 */
export function SettingsRow({
  label,
  htmlFor,
  alignTop = false,
  children,
}: {
  label: string
  htmlFor?: string
  alignTop?: boolean
  children: ReactNode
}) {
  const labelStyle: CSSProperties = alignTop
    ? { ...rowLabelStyle, alignSelf: 'start', paddingTop: '2px' }
    : rowLabelStyle
  return (
    <>
      {htmlFor !== undefined ? (
        <label htmlFor={htmlFor} style={labelStyle}>
          {label}
        </label>
      ) : (
        <span style={labelStyle}>{label}</span>
      )}
      <div style={controlCellStyle}>{children}</div>
    </>
  )
}

/** Full-width hairline separator between groups of rows. */
export function SettingsSeparator() {
  return (
    <div
      role="separator"
      style={{
        gridColumn: '1 / -1',
        borderTop: '1px solid var(--border-hairline, rgba(128, 128, 128, 0.25))',
        margin: '2px 0',
      }}
    />
  )
}

/** Faint explanatory footnote, aligned under the controls column. */
export function SettingsNote({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        gridColumn: '2',
        marginTop: '-6px',
        maxWidth: '36em',
        fontSize: '11px',
        lineHeight: 1.5,
        color: 'var(--text-faint, #888)',
      }}
    >
      {children}
    </div>
  )
}

/** Shared pop-up-button (select) styling for settings controls. */
export const settingsSelectStyle: CSSProperties = {
  minWidth: '180px',
  padding: '4px 8px',
  fontSize: '13px',
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '6px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
}

/**
 * Per-<option> styling. Many browsers/webviews render a <select>'s dropdown
 * popup with OS-native styling that ignores the parent element's color, so
 * each <option> needs its own explicit background/color too (some engines
 * respect it, closing the gap that styling only the <select> leaves open).
 */
export const settingsOptionStyle: CSSProperties = {
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
}
