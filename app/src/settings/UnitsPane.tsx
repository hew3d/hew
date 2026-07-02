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
 * Reusable from both the standalone Settings window (Tauri) and the in-app
 * modal fallback (web).
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

const SYSTEM_OPTIONS: { value: LengthSystem; label: string }[] = [
  { value: 'metric', label: 'Metric' },
  { value: 'imperial', label: 'Imperial' },
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

// Follow-up: many browsers/webviews render a <select>'s dropdown
// popup using OS-native styling that ignores the *parent* element's color
// (only `selectStyle` above), always showing dark-mode-only or light-mode-
// only text regardless of theme. Styling each <option> directly is the
// standard workaround — some engines respect it, closing the gap that
// styling only the <select> leaves open.
const optionStyle: React.CSSProperties = {
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
}

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
    <div>
      <h3 style={{ margin: '0 0 12px 0', fontSize: '14px', color: 'var(--text-primary, #eee)', fontWeight: 600 }}>
        Units
      </h3>

      <label style={labelStyle}>System</label>
      <select
        value={system}
        onChange={(e) => handleSystemChange(e.target.value as LengthSystem)}
        style={{ ...selectStyle, marginBottom: '12px' }}
      >
        {SYSTEM_OPTIONS.map((opt) => (
          <option key={opt.value} value={opt.value} style={optionStyle}>
            {opt.label}
          </option>
        ))}
      </select>

      <label style={labelStyle}>Format</label>
      <select
        value={format}
        onChange={(e) => setLengthUnit(e.target.value as LengthFormat)}
        style={selectStyle}
      >
        {formatOptions.map((value) => {
          const opt = LENGTH_FORMAT_OPTIONS.find((o) => o.value === value)
          return (
            <option key={value} value={value} style={optionStyle}>
              {opt?.label ?? value}
            </option>
          )
        })}
      </select>

      <p style={{ fontSize: '11px', color: 'var(--text-faint, #777)', marginTop: '10px', lineHeight: 1.4 }}>
        Controls how lengths are displayed (e.g. the Move tool's live measurement).
        Model geometry is always stored in meters internally.
      </p>
    </div>
  )
}
