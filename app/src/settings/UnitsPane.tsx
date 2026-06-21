/**
 * UnitsPane — the "Units" settings pane.
 *
 * A labeled selector bound to the length-unit singleton (app/src/settings/units.ts).
 * Reusable from both the standalone Settings window (Tauri) and the in-app
 * modal fallback (web).
 */

import { useEffect, useState } from 'react'
import { getLengthUnit, setLengthUnit, subscribe, LENGTH_UNIT_OPTIONS, type LengthUnit } from './units'

export function UnitsPane() {
  const [unit, setUnit] = useState<LengthUnit>(() => getLengthUnit())

  // Keep in sync with external changes (the other window, or another
  // subscriber in this same window).
  useEffect(() => subscribe(setUnit), [])

  return (
    <div>
      <h3 style={{ margin: '0 0 12px 0', fontSize: '14px', color: '#eee', fontWeight: 600 }}>
        Units
      </h3>
      <label style={{ display: 'block', fontSize: '12px', color: '#aaa', marginBottom: '6px' }}>
        Length unit
      </label>
      <select
        value={unit}
        onChange={(e) => setLengthUnit(e.target.value as LengthUnit)}
        style={{
          width: '100%',
          padding: '6px 8px',
          fontSize: '13px',
          background: '#2a2a2a',
          color: '#eee',
          border: '1px solid #444',
          borderRadius: '4px',
          fontFamily: 'system-ui, sans-serif',
        }}
      >
        {LENGTH_UNIT_OPTIONS.map((opt) => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>
      <p style={{ fontSize: '11px', color: '#777', marginTop: '10px', lineHeight: 1.4 }}>
        Controls how lengths are displayed (e.g. the Move tool's live measurement).
        Model geometry is always stored in meters internally.
      </p>
    </div>
  )
}
