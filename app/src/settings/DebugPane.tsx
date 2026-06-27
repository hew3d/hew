/**
 * DebugPane — the "Debug" settings pane (docs/DEVELOPMENT.md).
 *
 * A single checkbox bound to the Debug Mode singleton (settings/debugMode.ts).
 * The actual effects (file logging, input recording, kernel torture mode) are
 * applied by App.tsx/wasm/loader.ts subscribers — this pane only flips the
 * persisted setting and explains what it does.
 *
 * Reusable from both the standalone Settings window (Tauri) and the in-app
 * modal fallback (web).
 */

import { useEffect, useState } from 'react'
import { getDebugMode, setDebugMode, subscribe } from './debugMode'
import { isTauri } from '../io/fileHost'
import { downloadDiagnosticLog } from '../log/diagnosticLog'

const labelStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '8px',
  fontSize: '13px',
  color: '#ddd',
  cursor: 'pointer',
}

const buttonStyle: React.CSSProperties = {
  marginTop: '12px',
  padding: '6px 12px',
  fontSize: '13px',
  background: '#2a2a2a',
  color: '#eee',
  border: '1px solid #444',
  borderRadius: '4px',
  cursor: 'pointer',
}

export function DebugPane() {
  const [on, setOn] = useState<boolean>(() => getDebugMode())

  // Keep in sync with external changes (the other window, or another
  // subscriber in this same window).
  useEffect(() => subscribe(setOn), [])

  return (
    <div>
      <h3 style={{ margin: '0 0 12px 0', fontSize: '14px', color: '#eee', fontWeight: 600 }}>
        Debug
      </h3>

      <label style={labelStyle}>
        <input
          type="checkbox"
          checked={on}
          onChange={(e) => setDebugMode(e.target.checked)}
        />
        Enable Debug Mode
      </label>

      <p style={{ fontSize: '11px', color: '#777', marginTop: '10px', lineHeight: 1.4 }}>
        Debug Mode enables the rolling diagnostic log <strong>file</strong>, low-level
        input recording, and kernel <strong>torture mode</strong> (extra validation
        and a re-tessellation self-check after every op — slower, but surfaces
        flakes at the exact op).
      </p>

      {isTauri ? (
        <p style={{ fontSize: '11px', color: '#777', marginTop: '10px', lineHeight: 1.4 }}>
          Written to the app log directory as <code>diagnostic.log</code>.
        </p>
      ) : (
        <button style={buttonStyle} onClick={() => downloadDiagnosticLog()}>
          Download diagnostic log
        </button>
      )}
    </div>
  )
}
