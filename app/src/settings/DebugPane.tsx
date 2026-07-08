/**
 * DebugPane — the "Debug" settings pane (docs/DEVELOPMENT.md).
 *
 * A single checkbox bound to the Debug Mode singleton (settings/debugMode.ts).
 * The actual effects (file logging, input recording, kernel torture mode) are
 * applied by App.tsx/wasm/loader.ts subscribers — this pane only flips the
 * persisted setting and explains what it does.
 *
 * Laid out on the shared macOS-HIG form grid (SettingsForm.tsx). Reusable
 * from both the standalone Settings window (Tauri) and the in-app modal
 * fallback (web). Changes apply instantly — no OK/Cancel.
 */

import { useEffect, useState } from 'react'
import { getDebugMode, setDebugMode, subscribe } from './debugMode'
import { isTauri } from '../io/fileHost'
import { downloadDiagnosticLog } from '../log/diagnosticLog'
import { SettingsForm, SettingsRow, SettingsSeparator, SettingsNote } from './SettingsForm'

// The checkbox carries its own inline <label> (macOS checkboxes put the text
// to the right of the box), so the row label is a plain span (no htmlFor).
const checkboxLabelStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '7px',
  fontSize: '13px',
  color: 'var(--text-primary, #eee)',
  cursor: 'pointer',
}

const buttonStyle: React.CSSProperties = {
  padding: '4px 12px',
  fontSize: '13px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '6px',
  cursor: 'pointer',
}

export function DebugPane() {
  const [on, setOn] = useState<boolean>(() => getDebugMode())

  // Keep in sync with external changes (the other window, or another
  // subscriber in this same window).
  useEffect(() => subscribe(setOn), [])

  return (
    <SettingsForm>
      <SettingsRow label="Debug Mode" alignTop>
        <label style={checkboxLabelStyle}>
          <input
            type="checkbox"
            checked={on}
            onChange={(e) => setDebugMode(e.target.checked)}
            style={{ accentColor: 'var(--accent-base, #5b8cff)', margin: 0 }}
          />
          Enable Debug Mode
        </label>
      </SettingsRow>

      <SettingsNote>
        Debug Mode enables the rolling diagnostic log <strong>file</strong>, low-level
        input recording, and kernel <strong>torture mode</strong> (extra validation
        and a re-tessellation self-check after every op — slower, but surfaces
        flakes at the exact op).
      </SettingsNote>

      <SettingsSeparator />

      {isTauri ? (
        <SettingsNote>
          Written to the app log directory as <code>diagnostic.log</code>.
        </SettingsNote>
      ) : (
        <SettingsRow label="Diagnostics">
          <button style={buttonStyle} onClick={() => downloadDiagnosticLog()}>
            Download Diagnostic Log…
          </button>
        </SettingsRow>
      )}
    </SettingsForm>
  )
}
