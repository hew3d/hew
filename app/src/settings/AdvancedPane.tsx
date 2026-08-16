/**
 * AdvancedPane — the "Advanced" settings pane: which server "Open on Phone"
 * talks to (docs/design/self-hosting-relay.md §3). Hew cloud (app.hew3d.com)
 * or a self-hosted origin that serves both the Hew web app and its relay
 * under `/relay/` (docs/SELF_HOSTING.md), with an optional upload key and a
 * *Test connection* probe.
 *
 * All behavior lives in `useServerSettingForm` (serverForm.ts), shared with
 * the Windows mirror in FluentSettingsPage.tsx; this file is the macOS-HIG
 * rendering on the SettingsForm grid. In the browser build the setting does
 * not exist (whatever origin serves the app is its server) and the pane says
 * so read-only.
 */

import type { CSSProperties, KeyboardEvent } from 'react'
import { SettingsForm, SettingsNote, SettingsRow, SettingsSeparator } from './SettingsForm'
import { describeIdentity, useServerSettingForm } from './serverForm'
import { CLOUD_ORIGIN } from './server'

const radioLabelStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '7px',
  fontSize: '13px',
  color: 'var(--text-primary, #eee)',
  cursor: 'pointer',
}

const inputStyle: CSSProperties = {
  width: '340px',
  padding: '4px 8px',
  fontSize: '13px',
  fontFamily: 'var(--font-family-mono, ui-monospace, SFMono-Regular, monospace)',
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '6px',
  boxSizing: 'border-box',
}

function buttonStyle(disabled: boolean): CSSProperties {
  return {
    padding: '4px 12px',
    fontSize: '13px',
    fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
    background: 'var(--surface-input, #2a2a2a)',
    color: disabled ? 'var(--text-faint, #666)' : 'var(--text-primary, #eee)',
    border: '1px solid var(--border-strong, #444)',
    borderRadius: '6px',
    cursor: disabled ? 'default' : 'pointer',
  }
}

const errorStyle: CSSProperties = {
  gridColumn: '2',
  marginTop: '-6px',
  maxWidth: '36em',
  fontSize: '11px',
  lineHeight: 1.5,
  color: 'var(--danger-text, #e88)',
}

const okStyle: CSSProperties = {
  gridColumn: '2',
  marginTop: '-6px',
  maxWidth: '36em',
  fontSize: '11px',
  lineHeight: 1.5,
  color: 'var(--success-text, #8c8)',
}

export function AdvancedPane() {
  const form = useServerSettingForm()

  if (!form.available) {
    const origin = typeof window !== 'undefined' ? window.location.origin : ''
    return (
      <SettingsForm>
        <SettingsRow label="Server:">
          <span style={{ fontFamily: 'var(--font-family-mono, monospace)', fontSize: '12px' }} data-testid="settings-server-readonly">
            {origin}
          </span>
        </SettingsRow>
        <SettingsNote>
          In the browser, Hew talks to the server it was loaded from — there is nothing to configure here.
          The desktop app can be pointed at a self-hosted server under Settings ▸ Advanced.
        </SettingsNote>
      </SettingsForm>
    )
  }

  const selfHosted = form.draft.mode === 'self-hosted'
  const commitOnEnter = (e: KeyboardEvent<HTMLInputElement>): void => {
    if (e.key === 'Enter') {
      e.preventDefault()
      void form.commit()
    }
  }

  return (
    <SettingsForm>
      <SettingsRow label="Server:" alignTop>
        <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }} role="radiogroup" aria-label="Server">
          <label style={radioLabelStyle}>
            <input
              type="radio"
              name="settings-server-mode"
              value="cloud"
              checked={!selfHosted}
              onChange={() => form.setMode('cloud')}
              style={{ accentColor: 'var(--accent-base, #5b8cff)', margin: 0 }}
            />
            Hew cloud ({CLOUD_ORIGIN.replace(/^https:\/\//, '')})
          </label>
          <label style={radioLabelStyle}>
            <input
              type="radio"
              name="settings-server-mode"
              value="self-hosted"
              checked={selfHosted}
              onChange={() => form.setMode('self-hosted')}
              style={{ accentColor: 'var(--accent-base, #5b8cff)', margin: 0 }}
            />
            Self-hosted
          </label>
        </div>
      </SettingsRow>

      <SettingsRow label="Address:" htmlFor="settings-server-origin">
        <input
          id="settings-server-origin"
          type="url"
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          placeholder="https://hew.example.org"
          value={form.draft.origin === CLOUD_ORIGIN ? '' : form.draft.origin}
          disabled={!selfHosted}
          onChange={(e) => form.setOriginDraft(e.target.value)}
          onBlur={() => void form.commit()}
          onKeyDown={commitOnEnter}
          style={{ ...inputStyle, opacity: selfHosted ? 1 : 0.5 }}
        />
      </SettingsRow>
      {selfHosted && (
        <SettingsRow label="Upload key:" htmlFor="settings-server-upload-key">
          <input
            id="settings-server-upload-key"
            type="password"
            autoComplete="off"
            placeholder="none"
            value={form.draft.uploadKey}
            onChange={(e) => form.setUploadKeyDraft(e.target.value)}
            onBlur={() => void form.commit()}
            onKeyDown={commitOnEnter}
            style={inputStyle}
          />
        </SettingsRow>
      )}
      {selfHosted && (
        <SettingsNote>
          Only needed if the server's admin set one. Stored on this computer in plain text and only ever sent
          to the address above — never to the Hew cloud.
        </SettingsNote>
      )}
      {form.error !== null && (
        <div style={errorStyle} role="alert" data-testid="settings-server-error">
          {form.error}
        </div>
      )}

      <SettingsRow label="">
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
          <button
            type="button"
            style={buttonStyle(form.test.kind === 'testing')}
            disabled={form.test.kind === 'testing'}
            onClick={() => void form.testConnection()}
          >
            {form.test.kind === 'testing' ? 'Testing…' : 'Test connection'}
          </button>
        </div>
      </SettingsRow>
      {form.test.kind === 'ok' && (
        <div style={okStyle} role="status" data-testid="settings-server-test-ok">
          Reachable: {new URL(form.test.identity.origin).host} — {describeIdentity(form.test.identity)}
        </div>
      )}
      {form.test.kind === 'fail' && (
        <div style={errorStyle} role="alert" data-testid="settings-server-test-fail">
          {form.test.message}
        </div>
      )}

      <SettingsSeparator />

      <SettingsNote>
        A self-hosted server must serve the Hew web app too — the QR code points your phone at it — with the relay
        under <code>/relay/</code> (see the self-hosting guide). If it uses a certificate from your own authority,
        trust that authority on this computer and on the phone. Over plain <code>http://</code> the phone's in-app
        scanner can't open the camera; scan with the camera app instead.
      </SettingsNote>
    </SettingsForm>
  )
}
