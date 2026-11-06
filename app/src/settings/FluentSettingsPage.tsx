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
 * as the macOS panes (units.ts / theme.ts / debugMode.ts, and the
 * Rust-held server setting via serverForm.ts) — changes apply instantly and
 * sync across windows; no OK/Cancel.
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
import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'
import { useLingui } from '@lingui/react/macro'
import {
  SYSTEM_LANGUAGE,
  getLanguageSetting,
  setLanguageSetting,
  subscribe as subscribeLanguage,
  type LanguageSetting,
} from '../i18n/language'
import { languageOptions } from '../i18n/languageOptions'
import { libraryStore } from '../io/libraryStore'
import { describeIdentity, useServerSettingForm } from './serverForm'
import { CLOUD_ORIGIN } from './server'
import { isComposingKey } from '../input/keyboard'
import { ControlsPane } from './ControlsPane'
import { isRecordingChord } from '../keymap/keymap'

// Windows 11 app type ramp (Segoe UI Variable falls back to Segoe UI in
// webviews that don't expose the variable face).
// oxlint-disable-next-line lingui/no-unlocalized-strings -- CSS font stack
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

function fluentButtonStyle(disabled: boolean): CSSProperties {
  return {
    padding: '5px 14px',
    fontSize: '14px',
    fontFamily: FONT,
    background: 'var(--surface-input, #2a2a2a)',
    color: disabled ? 'var(--text-faint, #666)' : 'var(--text-primary, #eee)',
    border: '1px solid var(--border-strong, #444)',
    borderRadius: '4px',
    cursor: disabled ? 'default' : 'pointer',
    flexShrink: 0,
  }
}

/** Library folder path: a read-only text input holding the real absolute
 * path exactly as the backend reports it — no truncation trick that could
 * visually corrupt it (an RTL-flip ellipsis, tried earlier, silently
 * dropped the leading `/` of an absolute path in some renderers). */
const libraryPathInputStyle: CSSProperties = {
  flex: 1,
  minWidth: 0,
  padding: '5px 10px',
  fontFamily: 'var(--font-family-mono, ui-monospace, SFMono-Regular, monospace)',
  fontSize: '12px',
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '4px',
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
  const { t } = useLingui()
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: '10px', flexShrink: 0 }}>
      <span style={{ fontSize: '14px', color: 'var(--text-secondary, #ddd)' }}>
        {checked ? t`On` : t`Off`}
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

const SYSTEM_OPTIONS: { value: LengthSystem; label: MessageDescriptor }[] = [
  { value: 'metric', label: msg`Metric` },
  { value: 'imperial', label: msg`Imperial` },
]

// Windows 11 phrasing: apps offer "Use system setting" rather than "Auto".
const THEME_OPTIONS: { value: ThemeSetting; label: MessageDescriptor }[] = [
  { value: 'light', label: msg`Light` },
  { value: 'dark', label: msg`Dark` },
  { value: 'auto', label: msg`Use system setting` },
]

/** The "Server" section — the Windows mirror of AdvancedPane.tsx, on the
 *  same `useServerSettingForm` state machine (see serverForm.ts). */
function ServerSection() {
  const { t } = useLingui()
  const form = useServerSettingForm()
  const cloudHost = CLOUD_ORIGIN.replace(/^https:\/\//, '')
  const selfHosted = form.draft.mode === 'self-hosted'
  const reachableHost = form.test.kind === 'ok' ? new URL(form.test.identity.origin).host : ''
  const reachableDetails = form.test.kind === 'ok' ? describeIdentity(form.test.identity) : ''
  const commitOnEnter = (e: React.KeyboardEvent<HTMLInputElement>): void => {
    if (isComposingKey(e)) return
    if (e.key === 'Enter') {
      e.preventDefault()
      void form.commit()
    }
  }
  if (!form.available) {
    return (
      <>
        <div style={sectionHeaderStyle}>{t`Server`}</div>
        <SettingsCard
          title={t`Open on Phone server`}
          description={t`In the browser, Hew talks to the server it was loaded from — there is nothing to configure here.`}
        >
          <span style={{ ...libraryPathInputStyle, flex: 'none' }} data-testid="settings-server-readonly">
            {typeof window !== 'undefined' ? window.location.origin : ''}
          </span>
        </SettingsCard>
      </>
    )
  }
  return (
    <>
      <div style={sectionHeaderStyle}>{t`Server`}</div>
      <SettingsCard
        title={t`Open on Phone server`}
        description={t`Where the desktop uploads and where the QR code points your phone. A self-hosted server must serve the Hew web app with the relay under /relay/ (see the self-hosting guide).`}
        htmlFor="fluent-server-mode"
      >
        <select
          id="fluent-server-mode"
          value={form.draft.mode}
          onChange={(e) => form.setMode(e.target.value as 'cloud' | 'self-hosted')}
          style={selectStyle}
        >
          <option value="cloud" style={optionStyle}>
            {t`Hew cloud (${cloudHost})`}
          </option>
          <option value="self-hosted" style={optionStyle}>
            {t`Self-hosted`}
          </option>
        </select>
      </SettingsCard>
      {selfHosted && (
        <div style={cardStyle}>
          <label htmlFor="fluent-server-origin" style={{ ...cardTitleStyle, flexShrink: 0 }}>
            {t`Address:`}
          </label>
          <input
            id="fluent-server-origin"
            type="url"
            inputMode="url"
            autoComplete="off"
            spellCheck={false}
            placeholder="https://hew.example.org"
            value={form.draft.origin === CLOUD_ORIGIN ? '' : form.draft.origin}
            onChange={(e) => form.setOriginDraft(e.target.value)}
            onBlur={() => void form.commit()}
            onKeyDown={commitOnEnter}
            style={libraryPathInputStyle}
          />
        </div>
      )}
      {selfHosted && (
        <SettingsCard
          title={t`Upload key`}
          description={t`Only if the server's admin set one. Stored on this computer in plain text and only ever sent to the address above — never to the Hew cloud.`}
          htmlFor="fluent-server-upload-key"
        >
          <input
            id="fluent-server-upload-key"
            type="password"
            autoComplete="off"
            placeholder={t`none`}
            value={form.draft.uploadKey}
            onChange={(e) => form.setUploadKeyDraft(e.target.value)}
            onBlur={() => void form.commit()}
            onKeyDown={commitOnEnter}
            style={{ ...libraryPathInputStyle, flex: 'none', width: '260px' }}
          />
        </SettingsCard>
      )}
      {form.error !== null && (
        <div style={{ ...cardDescStyle, color: 'var(--danger-text, #e88)', margin: '2px 1px 6px' }} role="alert" data-testid="settings-server-error">
          {form.error}
        </div>
      )}
      <SettingsCard
        title={t`Test connection`}
        description={
          form.test.kind === 'ok'
            ? t`Reachable: ${reachableHost} — ${reachableDetails}`
            : form.test.kind === 'fail'
              ? form.test.message
              : selfHosted
                ? t`Checks that the address serves a Hew relay and, if it needs one, that your upload key is accepted. Over plain http:// the phone's in-app scanner can't open the camera; scan with the camera app instead. A certificate from your own authority must be trusted on this computer and on the phone.`
                : t`Checks that the Hew cloud relay is reachable from this computer.`
        }
      >
        <button
          type="button"
          onClick={() => void form.testConnection()}
          disabled={form.test.kind === 'testing'}
          style={fluentButtonStyle(form.test.kind === 'testing')}
        >
          {form.test.kind === 'testing' ? t`Testing…` : t`Test connection`}
        </button>
      </SettingsCard>
    </>
  )
}

export function FluentSettingsPage({ onBack }: { onBack: () => void }) {
  const { t, i18n } = useLingui()
  const [format, setFormat] = useState<LengthFormat>(() => getLengthUnit())
  const [theme, setTheme] = useState<ThemeSetting>(() => getThemeSetting())
  const [debug, setDebug] = useState<boolean>(() => getDebugMode())
  const [language, setLanguage] = useState<LanguageSetting>(() => getLanguageSetting())
  const libStore = libraryStore()
  const libraryAvailable = libStore.available()
  const [libraryPath, setLibraryPath] = useState<string | null>(null)

  useEffect(() => subscribeUnits(setFormat), [])
  useEffect(() => subscribeTheme(setTheme), [])
  useEffect(() => subscribeDebug(setDebug), [])
  useEffect(() => subscribeLanguage(setLanguage), [])

  useEffect(() => {
    if (!libraryAvailable) return
    let cancelled = false
    const refresh = (): void => {
      libStore
        .folderInfo()
        .then(({ path }) => {
          if (!cancelled) setLibraryPath(path)
        })
        .catch(() => {
          /* best-effort — leave the last known path displayed */
        })
    }
    refresh()
    const unsubscribe = libStore.subscribe(refresh)
    return () => {
      cancelled = true
      unsubscribe()
    }
    // `libStore` is the module-level singleton libraryStore() always
    // returns — its identity is stable, so it's deliberately not a dep.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [libraryAvailable])

  async function handleChooseLibraryFolder(): Promise<void> {
    if (!libraryAvailable) return
    try {
      const next = await libStore.chooseFolder()
      if (next !== null) setLibraryPath(next)
    } catch {
      /* cancelled or unsupported — leave the current path displayed */
    }
  }

  // Esc returns to the document — captured before the app's global keydown
  // handlers so an open settings page can't also cancel a tool underneath.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (isComposingKey(e)) return
      // Controls is recording a shortcut: its Escape cancels the recording.
      if (isRecordingChord()) return
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
  const langOptions = languageOptions({ systemLabel: t`System default`, debugMode: debug })

  function handleSystemChange(next: LengthSystem): void {
    if (next === system) return
    setLengthUnit(DEFAULT_FORMAT_FOR_SYSTEM[next])
  }

  return (
    <div style={pageStyle} role="region" aria-label={t`Settings`}>
      <div style={headerStyle}>
        <button
          style={backBtnStyle}
          onClick={onBack}
          aria-label={t`Back`}
          title={t`Back`}
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
        <h1 style={titleStyle}>{t`Settings`}</h1>
      </div>

      <div style={scrollStyle}>
        <div style={contentStyle}>
          <div style={sectionHeaderStyle}>{t`Units`}</div>
          <SettingsCard
            title={t`Measurement system`}
            description={t`Model geometry is always stored in meters internally.`}
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
                  {i18n._(opt.label)}
                </option>
              ))}
            </select>
          </SettingsCard>
          <SettingsCard
            title={t`Length format`}
            description={t`How lengths are displayed — e.g. the Move tool's live measurement.`}
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
                    {opt === undefined ? value : i18n._(opt.label)}
                  </option>
                )
              })}
            </select>
          </SettingsCard>

          <div style={sectionHeaderStyle}>{t`Controls`}</div>
          <div style={{ ...cardStyle, display: 'block' }}>
            <ControlsPane />
          </div>

          <div style={sectionHeaderStyle}>{t`Appearance`}</div>
          <SettingsCard title={t`App theme`} htmlFor="fluent-theme">
            <select
              id="fluent-theme"
              value={theme}
              onChange={(e) => setThemeSetting(e.target.value as ThemeSetting)}
              style={selectStyle}
            >
              {THEME_OPTIONS.map((opt) => (
                <option key={opt.value} value={opt.value} style={optionStyle}>
                  {i18n._(opt.label)}
                </option>
              ))}
            </select>
          </SettingsCard>

          <div style={sectionHeaderStyle}>{t`Language`}</div>
          <SettingsCard
            title={t`Language`}
            description={t`System default follows your OS language, when Hew is available in it.`}
            htmlFor="fluent-language"
          >
            <select
              id="fluent-language"
              value={langOptions.some((o) => o.value === language) ? language : SYSTEM_LANGUAGE}
              onChange={(e) => setLanguageSetting(e.target.value as LanguageSetting)}
              style={selectStyle}
            >
              {langOptions.map((opt) => (
                <option key={opt.value} value={opt.value} style={optionStyle}>
                  {opt.label}
                </option>
              ))}
            </select>
          </SettingsCard>

          <div style={sectionHeaderStyle}>{t`Library`}</div>
          <div style={cardStyle}>
            <label htmlFor="fluent-library-folder-path" style={{ ...cardTitleStyle, flexShrink: 0 }}>
              {t`Library folder:`}
            </label>
            <input
              id="fluent-library-folder-path"
              type="text"
              readOnly
              value={libraryAvailable ? (libraryPath ?? '') : ''}
              placeholder={libraryAvailable ? t`Loading…` : t`Not available in the browser build yet.`}
              title={libraryPath ?? undefined}
              style={libraryPathInputStyle}
            />
            <button
              type="button"
              onClick={handleChooseLibraryFolder}
              disabled={!libraryAvailable}
              style={fluentButtonStyle(!libraryAvailable)}
            >
              {t`Change…`}
            </button>
          </div>

          <ServerSection />

          <div style={sectionHeaderStyle}>{t`Debug`}</div>
          <SettingsCard
            title={t`Debug mode`}
            description={t`Enables the rolling diagnostic log file (diagnostic.log in the app log directory), low-level input recording, and kernel torture mode — extra validation after every op; slower, but surfaces flakes at the exact op.`}
          >
            <ToggleSwitch checked={debug} onChange={setDebugMode} ariaLabel={t`Debug mode`} />
          </SettingsCard>
        </div>
      </div>
    </div>
  )
}
