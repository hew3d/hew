/* oxlint-disable lingui/no-unlocalized-strings -- plugin install copy is English only until the feature graduates, and stays out of the catalogs. */
/**
 * ConsentSheet — what a plugin would be allowed to do, shown before it is
 * installed. A modal over whatever opened it (the Settings pane, or the
 * editor).
 *
 * Hew's lines and the plugin's words are kept apart. Everything above "The
 * plugin says" is Hew's: the permission lines, the warnings, the unsigned
 * line. The plugin's own explanation sits in its own block below, as plain
 * text, never markup, so it cannot pass for Hew speaking. Escape cancels;
 * Enter does nothing at all, so a stray keypress never installs; focus
 * starts on Cancel.
 */

import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { permissionText } from './permissionCopy'
import { buttonStyle } from './styles'
import type { ConsentPayload, PermissionLine } from './types'

/** The fixed line for a package no one has signed. Kept in step with
 *  `UNSIGNED_LINE` in `crates/plugin-host`. */
export const UNSIGNED_LINE = 'Unsigned. Hew cannot tell who made this.'

export interface ConsentSheetProps {
  consent: ConsentPayload
  onInstall: () => void
  onCancel: () => void
  /** An install is being written; the buttons wait. */
  busy?: boolean
  /** A refusal to show above the buttons (the sheet stays open). */
  error?: string | null
}

const sectionTitle: CSSProperties = {
  margin: '14px 0 4px',
  fontSize: '12px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
}

const listStyle: CSSProperties = { margin: 0, paddingLeft: '18px', lineHeight: 1.5 }

/** The most lines a list shows before "Show all"; a huge manifest must not
 *  push the warnings or the buttons out of reach. */
export const MAX_LINES_SHOWN = 12

function Lines({ lines, highlight }: { lines: PermissionLine[]; highlight?: boolean }) {
  const [all, setAll] = useState(false)
  const shown = all ? lines : lines.slice(0, MAX_LINES_SHOWN)
  return (
    <>
      <ul style={listStyle}>
        {shown.map((l) => (
          <li
            key={l.name}
            data-permission={l.name}
            style={highlight ? { background: 'var(--accent-tint-15, rgba(91,140,255,0.15))', borderRadius: '3px' } : undefined}
          >
            {permissionText(l.name, l.text)}
          </li>
        ))}
      </ul>
      {lines.length > shown.length && (
        <button type="button" onClick={() => setAll(true)} style={{ ...buttonStyle(), marginTop: '6px' }}>
          Show all {lines.length}
        </button>
      )}
    </>
  )
}

function Section({ heading, children }: { heading: string; children: ReactNode }) {
  return (
    <>
      <h3 style={sectionTitle}>{heading}</h3>
      {children}
    </>
  )
}

export function ConsentSheet({ consent, onInstall, onCancel, busy = false, error = null }: ConsentSheetProps) {
  const sheet = useRef<HTMLDivElement>(null)
  const cancel = useRef<HTMLButtonElement>(null)
  const onCancelRef = useRef(onCancel)
  onCancelRef.current = onCancel

  // Focus starts on Cancel and returns to what had it when the sheet closes.
  useEffect(() => {
    const before = document.activeElement
    cancel.current?.focus()
    return () => {
      if (before instanceof HTMLElement) before.focus()
    }
  }, [])

  // Keys are handled in the capture phase so the window behind (the web
  // Settings modal closes on Escape) never sees them.
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const root = sheet.current
      if (root === null) return
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onCancelRef.current()
      } else if (e.key === 'Enter') {
        // No accidental accept: Enter activates nothing here.
        e.preventDefault()
        e.stopPropagation()
      } else if (e.key === 'Tab') {
        const items = Array.from(root.querySelectorAll<HTMLElement>('button:not([disabled]), summary, [tabindex="0"]'))
        if (items.length === 0) return
        const first = items[0]
        const last = items[items.length - 1]
        const at = document.activeElement
        if (!root.contains(at)) {
          e.preventDefault()
          first.focus()
        } else if (e.shiftKey && at === first) {
          e.preventDefault()
          last.focus()
        } else if (!e.shiftKey && at === last) {
          e.preventDefault()
          first.focus()
        }
      }
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [])

  const { source, update } = consent
  const updating = update !== null
  const sourceText =
    source.kind === 'file' ? 'From file' : source.kind === 'dir' ? `Unpacked folder: ${source.path}` : `From ${source.kind}`

  return createPortal(
    <div
      onClick={(e) => e.stopPropagation()}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 500,
        background: 'var(--backdrop-dim, rgba(8,10,13,0.45))',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <div
        ref={sheet}
        role="dialog"
        aria-modal="true"
        aria-labelledby="plugin-consent-title"
        style={{
          width: 'min(460px, calc(100vw - 32px))',
          maxHeight: 'min(560px, calc(100vh - 32px))',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--surface-window, #1a1a1a)',
          color: 'var(--text-secondary, #ddd)',
          border: '1px solid var(--border-hairline, #333)',
          borderRadius: '10px',
          boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
          fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
          fontSize: '13px',
          userSelect: 'text',
        }}
      >
        <div style={{ padding: '16px 18px 4px', overflowY: 'auto', minHeight: 0, flex: 1 }}>
          <h2 id="plugin-consent-title" style={{ margin: 0, fontSize: '15px', color: 'var(--text-primary, #eee)' }}>
            {updating ? `Update ${consent.name}?` : `Install ${consent.name}?`}
          </h2>
          <div style={{ color: 'var(--text-faint, #888)', marginTop: '2px' }}>Version {consent.version}</div>
          <div style={{ marginTop: '10px' }}>
            <div data-consent="source">{sourceText}</div>
            {source.kind === 'file' && source.name !== '' && (
              <div style={{ color: 'var(--text-faint, #888)', fontSize: '12px' }}>{source.name}</div>
            )}
            {source.kind === 'dir' && (
              <div style={{ color: 'var(--text-faint, #888)', fontSize: '12px' }}>
                The files in this folder can change after you install.
              </div>
            )}
            {consent.unsigned && (
              <div data-consent="unsigned" style={{ marginTop: '4px', color: 'var(--text-primary, #eee)' }}>
                {UNSIGNED_LINE}
              </div>
            )}
            {consent.replaces !== null && (
              <div data-consent="replaces" style={{ marginTop: '4px', color: 'var(--text-primary, #eee)' }}>
                Replaces {consent.replaces.name} v{consent.replaces.version}. Hew cannot check it is the same author.
              </div>
            )}
          </div>

          {consent.warnings.length > 0 && (
            <div
              role="group"
              aria-label="Warnings from Hew"
              data-consent="warnings"
              style={{
                marginTop: '12px',
                padding: '8px 10px',
                border: '1px solid var(--status-warning, #e8c84a)',
                borderRadius: '6px',
                background: 'color-mix(in srgb, var(--status-warning, #e8c84a) 14%, transparent)',
                color: 'var(--text-primary, #eee)',
              }}
            >
              {consent.warnings.map((w) => (
                <div key={w}>{w}</div>
              ))}
            </div>
          )}

          <Section heading="It can">
            {consent.permissions.length > 0 ? (
              <Lines lines={consent.permissions} />
            ) : (
              <div style={{ color: 'var(--text-faint, #888)' }}>It asks for no permissions.</div>
            )}
          </Section>

          {consent.optional.length > 0 && (
            <Section heading="May ask for later">
              <Lines lines={consent.optional} />
            </Section>
          )}

          {consent.on_startup && (
            <div style={{ marginTop: '10px' }}>
              <strong style={{ color: 'var(--text-primary, #eee)' }}>Starts with Hew</strong>
              <span> It asks to run whenever Hew starts.</span>
            </div>
          )}

          {update !== null && (
            <Section heading={`New since ${update.from_version}`}>
              {update.added.length === 0 && update.new_optional.length === 0 && !update.startup_newly ? (
                <div style={{ color: 'var(--text-faint, #888)' }}>It asks for nothing new.</div>
              ) : (
                <>
                  {update.added.length > 0 && <Lines lines={update.added} highlight />}
                  {update.new_optional.length > 0 && (
                    <>
                      <div style={{ marginTop: '4px', color: 'var(--text-faint, #888)' }}>May now ask for later:</div>
                      <Lines lines={update.new_optional} highlight />
                    </>
                  )}
                  {update.startup_newly && <div>It now asks to start whenever Hew starts.</div>}
                </>
              )}
            </Section>
          )}

          {consent.activation_reason !== null && (
            <section
              aria-labelledby="plugin-consent-says"
              data-consent="plugin-says"
              style={{
                marginTop: '14px',
                padding: '8px 10px',
                borderLeft: '3px solid var(--border-strong, #444)',
                background: 'var(--surface-input, #2a2a2a)',
                borderRadius: '0 6px 6px 0',
              }}
            >
              <h3 id="plugin-consent-says" style={{ margin: '0 0 4px', fontSize: '12px', color: 'var(--text-faint, #888)' }}>
                The plugin says
              </h3>
              {/* Plain text. Never markup. */}
              <div style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{consent.activation_reason}</div>
            </section>
          )}
        </div>

        <div style={{ padding: '10px 18px 14px', flexShrink: 0 }}>
          {error !== null && (
            <div role="alert" style={{ marginBottom: '8px', color: 'var(--danger-base, #e0574f)' }}>
              {error}
            </div>
          )}
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '8px' }}>
            <button ref={cancel} type="button" onClick={onCancel} disabled={busy} style={buttonStyle(busy)}>
              Cancel
            </button>
            <button type="button" onClick={onInstall} disabled={busy} style={buttonStyle(busy, true)}>
              {updating ? 'Update' : 'Install'}
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  )
}
