/**
 * ThemePane — the "Theme" settings pane.
 *
 * A three-way Auto/Light/Dark selector bound to the theme singleton
 * (app/src/settings/theme.ts). 'Auto' follows the OS-level appearance;
 * explicit choices override it. `theme/applyTheme.ts` subscribes to the same
 * singleton and applies the resolved theme to the document — this pane only
 * flips the persisted setting.
 *
 * Laid out on the shared macOS-HIG form grid (SettingsForm.tsx). Reusable
 * from both the standalone Settings window (Tauri) and the in-app modal
 * fallback (web). Changes apply instantly — no OK/Cancel.
 */

import { useEffect, useState } from 'react'
import { getThemeSetting, setThemeSetting, subscribe, type ThemeSetting } from './theme'
import {
  SettingsForm,
  SettingsRow,
  SettingsNote,
  settingsSelectStyle,
  settingsOptionStyle,
} from './SettingsForm'

const THEME_OPTIONS: { value: ThemeSetting; label: string }[] = [
  { value: 'auto', label: 'Auto' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]

export function ThemePane() {
  const [setting, setSetting] = useState<ThemeSetting>(() => getThemeSetting())

  // Keep in sync with external changes (the other window, or another
  // subscriber in this same window).
  useEffect(() => subscribe(setSetting), [])

  return (
    <SettingsForm>
      <SettingsRow label="Appearance" htmlFor="settings-theme-appearance">
        <select
          id="settings-theme-appearance"
          value={setting}
          onChange={(e) => setThemeSetting(e.target.value as ThemeSetting)}
          style={settingsSelectStyle}
        >
          {THEME_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value} style={settingsOptionStyle}>
              {opt.label}
            </option>
          ))}
        </select>
      </SettingsRow>

      <SettingsNote>Auto follows your OS's light/dark appearance setting.</SettingsNote>
    </SettingsForm>
  )
}
