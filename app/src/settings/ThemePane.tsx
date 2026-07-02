/**
 * ThemePane — the "Theme" settings pane.
 *
 * A three-way Auto/Light/Dark selector bound to the theme singleton
 * (app/src/settings/theme.ts). 'Auto' follows the OS-level appearance;
 * explicit choices override it. `theme/applyTheme.ts` subscribes to the same
 * singleton and applies the resolved theme to the document — this pane only
 * flips the persisted setting.
 *
 * Reusable from both the standalone Settings window (Tauri) and the in-app
 * modal fallback (web).
 */

import { useEffect, useState } from 'react'
import { getThemeSetting, setThemeSetting, subscribe, type ThemeSetting } from './theme'

const THEME_OPTIONS: { value: ThemeSetting; label: string }[] = [
  { value: 'auto', label: 'Auto' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]

const selectStyle: React.CSSProperties = {
  width: '100%',
  padding: '6px 8px',
  fontSize: '13px',
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '4px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
}

const labelStyle: React.CSSProperties = {
  display: 'block',
  fontSize: '12px',
  color: 'var(--text-tertiary, #aaa)',
  marginBottom: '6px',
}

// Follow-up: (see UnitsPane.tsx's identical constant for the full
// explanation): many browsers/webviews render a <select>'s dropdown popup
// with OS-native styling that ignores the parent element's color, so each
// <option> needs its own explicit color too.
const optionStyle: React.CSSProperties = {
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
}

export function ThemePane() {
  const [setting, setSetting] = useState<ThemeSetting>(() => getThemeSetting())

  // Keep in sync with external changes (the other window, or another
  // subscriber in this same window).
  useEffect(() => subscribe(setSetting), [])

  return (
    <div>
      <h3 style={{ margin: '0 0 12px 0', fontSize: '14px', color: 'var(--text-primary, #eee)', fontWeight: 600 }}>
        Theme
      </h3>

      <label style={labelStyle}>Appearance</label>
      <select
        value={setting}
        onChange={(e) => setThemeSetting(e.target.value as ThemeSetting)}
        style={selectStyle}
      >
        {THEME_OPTIONS.map((opt) => (
          <option key={opt.value} value={opt.value} style={optionStyle}>
            {opt.label}
          </option>
        ))}
      </select>

      <p style={{ fontSize: '11px', color: 'var(--text-faint, #777)', marginTop: '10px', lineHeight: 1.4 }}>
        Auto follows your OS's light/dark appearance setting.
      </p>
    </div>
  )
}
