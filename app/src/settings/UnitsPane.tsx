/**
 * UnitsPane — the "Units" settings pane.
 *
 * Two-level system × format selector bound to the length-format
 * singleton (app/src/settings/units.ts):
 *   - A SYSTEM selector (Metric / Imperial).
 *   - A FORMAT selector whose options depend on the chosen system.
 * Changing system selects that system's default format (Meters for Metric,
 * Architectural for Imperial).
 *
 * Laid out on the shared macOS-HIG form grid (SettingsForm.tsx). Reusable
 * from both the standalone Settings window (Tauri) and the in-app modal
 * fallback (web). Changes apply instantly — no OK/Cancel.
 */

import { useEffect, useState } from 'react'
import {
  getLengthUnit,
  setLengthUnit,
  subscribe,
  LENGTH_FORMAT_OPTIONS,
  LENGTH_FORMATS_BY_SYSTEM,
  LENGTH_SYSTEM_OF,
  DEFAULT_FORMAT_FOR_SYSTEM,
  type LengthFormat,
  type LengthSystem,
} from './units'
import {
  SettingsForm,
  SettingsRow,
  SettingsSeparator,
  SettingsNote,
  settingsSelectStyle,
  settingsOptionStyle,
} from './SettingsForm'

const SYSTEM_OPTIONS: { value: LengthSystem; label: string }[] = [
  { value: 'metric', label: 'Metric' },
  { value: 'imperial', label: 'Imperial' },
]

export function UnitsPane() {
  const [format, setFormat] = useState<LengthFormat>(() => getLengthUnit())

  // Keep in sync with external changes (the other window, or another
  // subscriber in this same window).
  useEffect(() => subscribe(setFormat), [])

  const system = LENGTH_SYSTEM_OF[format]
  const formatOptions = LENGTH_FORMATS_BY_SYSTEM[system]

  function handleSystemChange(next: LengthSystem): void {
    if (next === system) return
    setLengthUnit(DEFAULT_FORMAT_FOR_SYSTEM[next])
  }

  return (
    <SettingsForm>
      <SettingsRow label="System" htmlFor="settings-units-system">
        <select
          id="settings-units-system"
          value={system}
          onChange={(e) => handleSystemChange(e.target.value as LengthSystem)}
          style={settingsSelectStyle}
        >
          {SYSTEM_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value} style={settingsOptionStyle}>
              {opt.label}
            </option>
          ))}
        </select>
      </SettingsRow>

      <SettingsRow label="Format" htmlFor="settings-units-format">
        <select
          id="settings-units-format"
          value={format}
          onChange={(e) => setLengthUnit(e.target.value as LengthFormat)}
          style={settingsSelectStyle}
        >
          {formatOptions.map((value) => {
            const opt = LENGTH_FORMAT_OPTIONS.find((o) => o.value === value)
            return (
              <option key={value} value={value} style={settingsOptionStyle}>
                {opt?.label ?? value}
              </option>
            )
          })}
        </select>
      </SettingsRow>

      <SettingsSeparator />

      <SettingsNote>
        Controls how lengths are displayed (e.g. the Move tool's live measurement).
        Model geometry is always stored in meters internally.
      </SettingsNote>
    </SettingsForm>
  )
}
