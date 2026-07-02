/**
 * SettingsWindow — standalone shell rendered in the separate Settings webview
 * window (Tauri) when the URL hash is `#settings` (see main.tsx).
 *
 * Two-column layout: a left list of setting categories ("Units", "Theme",
 * "Debug") and the right pane rendering the selected category's content.
 * Token-styled `
 * unconditionally, so this window's `data-theme` was always correct — it was
 * just ignored, since every color here was hardcoded dark regardless).
 */

import { useState } from 'react'
import { UnitsPane } from './UnitsPane'
import { ThemePane } from './ThemePane'
import { DebugPane } from './DebugPane'

type Category = 'units' | 'theme' | 'debug'

const CATEGORIES: { id: Category; label: string }[] = [
  { id: 'units', label: 'Units' },
  { id: 'theme', label: 'Theme' },
  { id: 'debug', label: 'Debug' },
]

export function SettingsWindow() {
  const [active, setActive] = useState<Category>('units')

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100vh',
        background: 'var(--surface-window, #1a1a1a)',
        color: 'var(--text-secondary, #ddd)',
        fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
        boxSizing: 'border-box',
      }}
    >
      <div
        style={{
          padding: '10px 16px',
          fontSize: '14px',
          fontWeight: 600,
          color: 'var(--text-primary, #eee)',
          borderBottom: '1px solid var(--border-hairline, #333)',
          flexShrink: 0,
        }}
      >
        Settings
      </div>
      <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        {/* Left list */}
        <div
          style={{
            width: '140px',
            flexShrink: 0,
            borderRight: '1px solid var(--border-hairline, #333)',
            background: 'var(--surface-panel, #202020)',
            padding: '8px 0',
          }}
        >
          {CATEGORIES.map((cat) => (
            <div
              key={cat.id}
              onClick={() => setActive(cat.id)}
              style={{
                padding: '6px 16px',
                fontSize: '13px',
                cursor: 'pointer',
                color: active === cat.id ? 'var(--accent-text-on-tint, #fff)' : 'var(--text-tertiary, #bbb)',
                background: active === cat.id ? 'var(--accent-tint-15, #3a5e9e)' : 'transparent',
              }}
            >
              {cat.label}
            </div>
          ))}
        </div>

        {/* Right pane */}
        <div style={{ flex: 1, minWidth: 0, padding: '16px', overflowY: 'auto' }}>
          {active === 'units' && <UnitsPane />}
          {active === 'theme' && <ThemePane />}
          {active === 'debug' && <DebugPane />}
        </div>
      </div>
    </div>
  )
}
