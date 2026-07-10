/**
 * FluentSettingsPage — the Windows settings surface: a full-window, in-app
 * page in the Windows 11 "app settings" idiom (Notepad, Paint, Photos): a
 * back arrow + "Settings" heading up top, then full-width settings cards —
 * label and description on the left, control on the right — grouped under
 * small section headers. Segoe UI Variable type, 4px corner radii, and a
 * toggle switch (not a checkbox) for on/off settings, per Fluent.
 *
 * Windows uses this instead of the separate Settings webview window that
 * macOS keeps (macOS HIG expects a standalone settings window; Windows 11
 * apps settled on the in-window page). Rendered by App when `openSettings`
 * runs on the Windows desktop shell; Esc or the back arrow returns to the
 * document.
 *
 * All colors come from theme tokens (app/src/theme/tokens.css) so light and
 * dark themes both render correctly. Bound to the same settings singletons
 * as the macOS panes (units.ts / theme.ts / debugMode.ts) — changes apply
 * instantly and sync across windows; no OK/Cancel.
 */

import { useEffect, useState, type CSSProperties, type ReactNode } from 'react'
import {
  getLengthUnit,
  setLengthUnit,
  subscribe as subscribeUnits,
  LENGTH_FORMAT_OPTIONS,
  LENGTH_FORMATS_BY_SYSTEM,
  LENGTH_SYSTEM_OF,
  DEFAULT_FORMAT_FOR_SYSTEM,
  type LengthFormat,
  type LengthSystem,
} from './units'
import {
  getThemeSetting,
  setThemeSetting,
  subscribe as subscribeTheme,
  type ThemeSetting,
} from './theme'
import { getDebugMode, setDebugMode, subscribe as subscribeDebug } from './debugMode'

// Windows 11 app type ramp (Segoe UI Variable falls back to Segoe UI in
// webviews that don't expose the variable face).
const FONT = "'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"

const pageStyle: CSSProperties = {
  position: 'fixed',
  inset: 0,
  zIndex: 3000,
  display: 'flex',
  flexDirection: 'column',
  background: 'var(--surface-window, #1a1a1a)',
  color: 'var(--text-secondary, #ddd)',
  fontFamily: FONT,
  fontSize: '14px',
}

const headerStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '14px',
  padding: '14px 24px 6px',
  flexShrink: 0,
}

const backBtnStyle: CSSProperties = {
  width: 36,
  height: 32,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  border: 'none',
  borderRadius: '4px',
  background: 'transparent',
  color: 'var(--text-primary, #eee)',
  cursor: 'pointer',
  padding: 0,
}

const titleStyle: CSSProperties = {
  fontSize: '28px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  lineHeight: 1.2,
  margin: 0,
}

const scrollStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  overflowY: 'auto',
  padding: '10px 24px 32px',
}

const contentStyle: CSSProperties = {
  maxWidth: '1000px',
  display: 'flex',
  flexDirection: 'column',
  gap: '4px',
}

const sectionHeaderStyle: CSSProperties = {
  fontSize: '14px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  margin: '18px 0 6px 1px',
}

const cardStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '16px',
  minHeight: '48px',
  padding: '12px 16px',
  background: 'var(--surface-bar, #202020)',
  border: '1px solid var(--border-hairline, rgba(128, 128, 128, 0.25))',
  borderRadius: '4px',
  marginBottom: '4px',
}

const cardTextStyle: CSSProperties = {
  flex: 1,
  minWidth: 0,
  display: 'flex',
  flexDirection: 'column',
  gap: '2px',
}

const cardTitleStyle: CSSProperties = {
  fontSize: '14px',
  color: 'var(--text-primary, #eee)',
}

const cardDescStyle: CSSProperties = {
  fontSize: '12px',
  lineHeight: 1.45,
  color: 'var(--text-faint, #888)',
}

const selectStyle: CSSProperties = {
  minWidth: '200px',
  padding: '5px 10px',
  fontSize: '14px',
  fontFamily: FONT,
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '4px',
  flexShrink: 0,
}

const optionStyle: CSSProperties = {
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
}

/** One Fluent settings card: title + optional description left, control right. */
function SettingsCard({
  title,
  description,
  htmlFor,
  children,
}: {
  title: string
  description?: string
  htmlFor?: string
  children: ReactNode
}) {
  return (
    <div style={cardStyle}>
      <div style={cardTextStyle}>
        {htmlFor !== undefined ? (
          <label htmlFor={htmlFor} style={cardTitleStyle}>
            {title}
          </label>
        ) : (
          <span style={cardTitleStyle}>{title}</span>
        )}
        {description !== undefined && <span style={cardDescStyle}>{description}</span>}
      </div>
      {children}
    </div>
  )
}

/** Windows 11 toggle switch (40×20 track, sliding thumb, On/Off caption). */
function ToggleSwitch({
  checked,
  onChange,
  ariaLabel,
}: {
  checked: boolean
  onChange: (next: boolean) => void
  ariaLabel: string
}) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: '10px', flexShrink: 0 }}>
      <span style={{ fontSize: '14px', color: 'var(--text-secondary, #ddd)' }}>
        {checked ? 'On' : 'Off'}
      </span>
      <button
        role="switch"
        aria-checked={checked}
        aria-label={ariaLabel}
        onClick={() => onChange(!checked)}
        style={{
          width: 40,
          height: 20,
          borderRadius: 10,
          border: checked
            ? '1px solid var(--accent-base, #5b8cff)'
            : '1px solid var(--border-strong, #666)',
          background: checked ? 'var(--accent-base, #5b8cff)' : 'transparent',
          position: 'relative',
          cursor: 'pointer',
          padding: 0,
          transition: 'background 0.15s ease',
        }}
      >
        <span
          style={{
            position: 'absolute',
            top: '50%',
            transform: 'translateY(-50%)',
            left: checked ? 22 : 3,
            width: 12,
            height: 12,
            borderRadius: '50%',
            background: checked ? 'var(--surface-window, #fff)' : 'var(--text-tertiary, #aaa)',
            transition: 'left 0.15s ease',
          }}
        />
      </button>
    </div>
  )
}

const SYSTEM_OPTIONS: { value: LengthSystem; label: string }[] = [
  { value: 'metric', label: 'Metric' },
  { value: 'imperial', label: 'Imperial' },
]

// Windows 11 phrasing: apps offer "Use system setting" rather than "Auto".
const THEME_OPTIONS: { value: ThemeSetting; label: string }[] = [
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
  { value: 'auto', label: 'Use system setting' },
]

export function FluentSettingsPage({ onBack }: { onBack: () => void }) {
  const [format, setFormat] = useState<LengthFormat>(() => getLengthUnit())
  const [theme, setTheme] = useState<ThemeSetting>(() => getThemeSetting())
  const [debug, setDebug] = useState<boolean>(() => getDebugMode())

  useEffect(() => subscribeUnits(setFormat), [])
  useEffect(() => subscribeTheme(setTheme), [])
  useEffect(() => subscribeDebug(setDebug), [])

  // Esc returns to the document — captured before the app's global keydown
  // handlers so an open settings page can't also cancel a tool underneath.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onBack()
      }
    }
    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
  }, [onBack])

  const system = LENGTH_SYSTEM_OF[format]
  const formatOptions = LENGTH_FORMATS_BY_SYSTEM[system]

  function handleSystemChange(next: LengthSystem): void {
    if (next === system) return
    setLengthUnit(DEFAULT_FORMAT_FOR_SYSTEM[next])
  }

  return (
    <div style={pageStyle} role="region" aria-label="Settings">
      <div style={headerStyle}>
        <button
          style={backBtnStyle}
          onClick={onBack}
          aria-label="Back"
          title="Back"
          autoFocus
          onMouseEnter={(e) => {
            e.currentTarget.style.background = 'var(--accent-tint-15, rgba(128,128,128,0.15))'
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.background = 'transparent'
          }}
        >
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
            <path d="M7.5 3 2.8 8l4.7 5M3.2 8h10" />
          </svg>
        </button>
        <h1 style={titleStyle}>Settings</h1>
      </div>

      <div style={scrollStyle}>
        <div style={contentStyle}>
          <div style={sectionHeaderStyle}>Units</div>
          <SettingsCard
            title="Measurement system"
            description="Model geometry is always stored in meters internally."
            htmlFor="fluent-units-system"
          >
            <select
              id="fluent-units-system"
              value={system}
              onChange={(e) => handleSystemChange(e.target.value as LengthSystem)}
              style={selectStyle}
            >
              {SYSTEM_OPTIONS.map((opt) => (
                <option key={opt.value} value={opt.value} style={optionStyle}>
                  {opt.label}
                </option>
              ))}
            </select>
          </SettingsCard>
          <SettingsCard
            title="Length format"
            description="How lengths are displayed — e.g. the Move tool's live measurement."
            htmlFor="fluent-units-format"
          >
            <select
              id="fluent-units-format"
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
          </SettingsCard>

          <div style={sectionHeaderStyle}>Appearance</div>
          <SettingsCard title="App theme" htmlFor="fluent-theme">
            <select
              id="fluent-theme"
              value={theme}
              onChange={(e) => setThemeSetting(e.target.value as ThemeSetting)}
              style={selectStyle}
            >
              {THEME_OPTIONS.map((opt) => (
                <option key={opt.value} value={opt.value} style={optionStyle}>
                  {opt.label}
                </option>
              ))}
            </select>
          </SettingsCard>

          <div style={sectionHeaderStyle}>Debug</div>
          <SettingsCard
            title="Debug mode"
            description="Enables the rolling diagnostic log file (diagnostic.log in the app log directory), low-level input recording, and kernel torture mode — extra validation after every op; slower, but surfaces flakes at the exact op."
          >
            <ToggleSwitch checked={debug} onChange={setDebugMode} ariaLabel="Debug mode" />
          </SettingsCard>
        </div>
      </div>
    </div>
  )
}
