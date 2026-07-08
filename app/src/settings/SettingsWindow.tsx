/**
 * SettingsWindow — standalone shell rendered in the separate Settings webview
 * window (Tauri) when the URL hash is `#settings` (see main.tsx).
 *
 * macOS-HIG preferences layout (kept identical on Windows/Linux — the style
 * is neutral enough everywhere): a top toolbar-style tab strip with one tab
 * per pane (icon above a short label, centered horizontally, active tab in a
 * subtle rounded-rect highlight like macOS toolbar tabs), and the selected
 * pane below in the classic right-aligned-labels form grid (SettingsForm.tsx).
 * No OK/Cancel — every control applies instantly via its settings singleton.
 *
 * All colors come from theme tokens (app/src/theme/tokens.css) so both light
 * and dark themes render correctly; icons are inline monochrome stroke SVGs
 * on `currentColor` (same hand-drawn stroke style as TitleBar.tsx's glyphs).
 */

import { useState, type CSSProperties, type ReactElement } from 'react'
import { UnitsPane } from './UnitsPane'
import { ThemePane } from './ThemePane'
import { DebugPane } from './DebugPane'

type Category = 'units' | 'theme' | 'debug'

const iconProps = {
  width: 22,
  height: 22,
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.5,
  strokeLinecap: 'round',
  strokeLinejoin: 'round',
  'aria-hidden': true,
} as const

/** Ruler — Units. */
function UnitsIcon() {
  return (
    <svg {...iconProps}>
      <rect x="2.5" y="8.5" width="19" height="7" rx="1.5" />
      <path d="M6.3 8.5v3.2M10.1 8.5v4.6M13.9 8.5v3.2M17.7 8.5v4.6" />
    </svg>
  )
}

/** Half-filled circle (appearance) — Theme. */
function ThemeIcon() {
  return (
    <svg {...iconProps}>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 3.5a8.5 8.5 0 0 1 0 17z" fill="currentColor" stroke="none" />
    </svg>
  )
}

/** Bug — Debug. */
function DebugIcon() {
  return (
    <svg {...iconProps}>
      <path d="M9.5 8.5a2.5 2.5 0 0 1 5 0" />
      <ellipse cx="12" cy="13.5" rx="4" ry="5" />
      <path d="M8 12H4.5M8 15.5l-3 1.8M16 12h3.5M16 15.5l3 1.8M9.9 8 7.8 5.6M14.1 8l2.1-2.4" />
    </svg>
  )
}

const TABS: { id: Category; label: string; icon: ReactElement }[] = [
  { id: 'units', label: 'Units', icon: <UnitsIcon /> },
  { id: 'theme', label: 'Theme', icon: <ThemeIcon /> },
  { id: 'debug', label: 'Debug', icon: <DebugIcon /> },
]

const tabStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  gap: '3px',
  minWidth: '64px',
  padding: '6px 10px 5px',
  border: 'none',
  borderRadius: 'var(--radius-control, 7px)',
  background: 'transparent',
  color: 'var(--text-tertiary, #bbb)',
  fontFamily: 'inherit',
  fontSize: '11px',
  lineHeight: 1.2,
  cursor: 'pointer',
}

const activeTabStyle: CSSProperties = {
  ...tabStyle,
  background: 'var(--accent-tint-15, rgba(91, 140, 255, 0.15))',
  color: 'var(--accent-text-on-tint, #bcd2ff)',
}

export function SettingsWindow() {
  const [active, setActive] = useState<Category>('units')

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        // 100% (not 100vh): the dedicated settings window fills the viewport
        // through the html/body/#root height chain anyway, and the web build
        // embeds this same component inside a fixed-height modal box — a
        // viewport unit there overflows the clipped box and the tab panel's
        // scrollbar can never engage.
        height: '100%',
        background: 'var(--surface-window, #1a1a1a)',
        color: 'var(--text-secondary, #ddd)',
        fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
        fontSize: '13px',
        boxSizing: 'border-box',
        userSelect: 'none',
      }}
    >
      {/* Toolbar-style tab strip (icon above label, centered). */}
      <div
        role="tablist"
        aria-label="Settings"
        style={{
          display: 'flex',
          justifyContent: 'center',
          gap: '2px',
          padding: '7px 12px',
          background: 'var(--surface-bar, #202020)',
          borderBottom: '1px solid var(--border-hairline, rgba(128, 128, 128, 0.25))',
          flexShrink: 0,
        }}
      >
        {TABS.map((tab) => {
          const selected = active === tab.id
          return (
            <button
              key={tab.id}
              type="button"
              role="tab"
              id={`settings-tab-${tab.id}`}
              aria-selected={selected}
              aria-controls={`settings-pane-${tab.id}`}
              onClick={() => setActive(tab.id)}
              style={selected ? activeTabStyle : tabStyle}
            >
              {tab.icon}
              <span>{tab.label}</span>
            </button>
          )
        })}
      </div>

      {/* Active pane (classic form grid inside — see SettingsForm.tsx). */}
      <div
        role="tabpanel"
        id={`settings-pane-${active}`}
        aria-labelledby={`settings-tab-${active}`}
        style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: '20px 24px 22px' }}
      >
        {active === 'units' && <UnitsPane />}
        {active === 'theme' && <ThemePane />}
        {active === 'debug' && <DebugPane />}
      </div>
    </div>
  )
}
