/* oxlint-disable lingui/no-unlocalized-strings -- plugin install copy is English only until the feature graduates, and stays out of the catalogs. */
/**
 * ConsentSheet — what a plugin would be allowed to do, shown before it is
 * installed. A modal over whatever opened it (the Settings pane, or the
 * editor).
 *
 * Hew's lines and the plugin's words are kept apart. Everything above "The
 * plugin says" is Hew's: the permission lines, the warnings, the unsigned
 * line. The plugin's own explanation sits in its own block below, as plain
 * text, never markup, so it cannot pass for Hew speaking. Escape cancels
 * (and does nothing while an install is being written, as Cancel does not);
 * Enter does nothing at all, so a stray keypress never installs; focus
 * starts on Cancel. The plugin's cleaned name and version are always shown.
 *
 * In grant mode (`grant`, the lines the kernel gave for a change to a
 * plugin's optional permissions) it asks "Allow more for this plugin?" and
 * shows only the plugin's cleaned name and version, what the change would
 * add and Hew's warnings; it accepts with "Allow". Cancel has focus and
 * Enter accepts nothing, as everywhere here.
 *
 * In review mode (`consent.review`, a folder plugin whose manifest now asks
 * for something different) it asks "Allow this plugin's changes?", says so
 * in place of what it replaces, lists what it now asks for and what it no
 * longer asks for, and accepts with "Allow".
 */

import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { permissionText } from './permissionCopy'
import { PathText } from './pathText'
import { cleanText, MAX_NAME_CHARS } from './pluginText'
import { buttonStyle } from './styles'
import type { ConsentPayload, PermissionLine } from './types'

/** The fixed line for a package no one has signed. Kept in step with
 *  `UNSIGNED_LINE` in `crates/plugin-host`. */
export const UNSIGNED_LINE = 'Unsigned. Hew cannot tell who made this.'

/** The review's line in place of the replaces line. Kept in step with
 *  `REVIEW_LINE` in `crates/plugin-host`. */
export const REVIEW_LINE = "This plugin's folder now asks for different permissions."

export interface ConsentSheetProps {
  consent: ConsentPayload
  onInstall: () => void
  onCancel: () => void
  /** An install is being written; the buttons wait. */
  busy?: boolean
  /** A refusal to show above the buttons (the sheet stays open). */
  error?: string | null
  /** Grant mode: what a change to the plugin's permissions would add, and
   *  the warnings its result triggers, as the kernel wrote them. Only
   *  `consent.name` and `consent.version` are used besides. */
  grant?: { added: PermissionLine[]; warnings: string[] }
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

export function ConsentSheet({ consent, onInstall, onCancel, busy = false, error = null, grant }: ConsentSheetProps) {
  const sheet = useRef<HTMLDivElement>(null)
  const cancel = useRef<HTMLButtonElement>(null)
  const onCancelRef = useRef(onCancel)
  onCancelRef.current = onCancel
  const busyRef = useRef(busy)
  busyRef.current = busy

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
        // While the install is being written there is nothing to cancel.
        if (!busyRef.current) onCancelRef.current()
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
  const review = consent.review === true
  const updating = update !== null
  // The same version over itself: a reinstall, not an update.
  const again = !review && consent.replaces?.version === consent.version
  // A review lists what changed, then what the plugin keeps: the full list
  // would say the new permissions twice.
  const reviewChanges = review && update !== null
  const without = (lines: PermissionLine[], added: PermissionLine[]): PermissionLine[] =>
    reviewChanges ? lines.filter((l) => !added.some((a) => a.name === l.name)) : lines
  const keptPermissions = without(consent.permissions, update?.added ?? [])
  const keptOptional = without(consent.optional, update?.new_optional ?? [])
  const keptSections = (
    <>
      {(!reviewChanges || keptPermissions.length > 0) && (
        <Section heading={reviewChanges ? 'It keeps' : 'It can'}>
          {keptPermissions.length > 0 ? (
            <Lines lines={keptPermissions} />
          ) : (
            <div style={{ color: 'var(--text-faint, #888)' }}>It asks for no permissions.</div>
          )}
        </Section>
      )}
      {keptOptional.length > 0 && (
        <Section heading="May ask for later">
          <Lines lines={keptOptional} />
        </Section>
      )}
    </>
  )
  const removed = update?.removed ?? []
  const removedOptional = update?.removed_optional ?? []
  // A folder copied in on the web is a file the store marks as a folder.
  const copied = source.kind === 'file' && source.origin === 'folder'
  const sourceText = copied ? 'Copied from a folder' : source.kind === 'file' ? 'From file' : source.kind === 'dir' ? 'Developer folder' : `From ${source.kind}`
  const granting = grant !== undefined
  const title = granting
    ? 'Allow more for this plugin?'
    : review
    ? "Allow this plugin's changes?"
    : again
      ? `Install ${consent.name} again?`
      : updating
        ? `Update ${consent.name}?`
        : `Install ${consent.name}?`
  const accept = review || granting ? 'Allow' : updating && !again ? 'Update' : 'Install'

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
        data-plugin-sheet=""
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
            {title}
          </h2>
          <div style={{ color: 'var(--text-faint, #888)', marginTop: '2px', overflowWrap: 'anywhere' }}>
            {review ? `${consent.name}, version ${consent.version}` : `Version ${consent.version}`}
          </div>
          {granting && (
            <>
              <div data-consent="grant-plugin" style={{ marginTop: '8px' }}>
                <PathText label="Plugin" value={consent.name} max={MAX_NAME_CHARS} />
              </div>
              <Section heading="It would also be able to">
                {grant.added.length > 0 ? (
                  <Lines lines={grant.added} highlight />
                ) : (
                  <div style={{ color: 'var(--text-faint, #888)' }}>Nothing new.</div>
                )}
              </Section>
              {grant.warnings.length > 0 && (
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
                  {grant.warnings.map((w) => (
                    <div key={w}>{w}</div>
                  ))}
                </div>
              )}
            </>
          )}
          {!granting && (<>
          <div style={{ marginTop: '10px' }}>
            <div data-consent="source">{sourceText}</div>
            {copied && source.name !== '' && <PathText label="Folder name" value={source.name} />}
            {source.kind === 'file' && !copied && source.name !== '' && (
              <div style={{ color: 'var(--text-faint, #888)', fontSize: '12px' }}>{cleanText(source.name, 128)}</div>
            )}
            {source.kind === 'dir' && <PathText label="Folder" value={source.path} max={512} />}
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
            {review && (
              <div data-consent="review" style={{ marginTop: '4px', color: 'var(--text-primary, #eee)', whiteSpace: 'pre-line', overflowWrap: 'anywhere' }}>
                {/* Its values (a command, another plugin's name) are on lines of their own. */}
                {typeof consent.review_line === 'string' && consent.review_line !== '' ? consent.review_line : REVIEW_LINE}
              </div>
            )}
            {review && consent.replaces !== null && consent.replaces.name !== consent.name && (
              <div data-consent="was-named" style={{ marginTop: '4px' }}>
                <PathText label="It was named" value={consent.replaces.name} max={MAX_NAME_CHARS} />
              </div>
            )}
            {!review && consent.replaces !== null && (
              <div data-consent="replaces" style={{ marginTop: '4px', color: 'var(--text-primary, #eee)' }}>
                {again
                  ? `Replaces the installed ${consent.replaces.name} v${consent.replaces.version} with this copy.`
                  : `Replaces ${consent.replaces.name} v${consent.replaces.version}. Hew cannot check it is the same author.`}
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

          {!reviewChanges && keptSections}

          {consent.on_startup && (
            <div style={{ marginTop: '10px' }}>
              <strong style={{ color: 'var(--text-primary, #eee)' }}>Starts with Hew</strong>
              <span> It asks to run whenever Hew starts.</span>
            </div>
          )}

          {update !== null && (
            <Section heading={review ? 'Now asks for' : `New since ${update.from_version}`}>
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

          {(removed.length > 0 || removedOptional.length > 0) && (
            <Section heading="No longer asks for">
              <div data-consent="removed">
                {removed.length > 0 && <Lines lines={removed} />}
                {removedOptional.length > 0 && (
                  <>
                    <div style={{ marginTop: '4px', color: 'var(--text-faint, #888)' }}>No longer may ask for later:</div>
                    <Lines lines={removedOptional} />
                  </>
                )}
              </div>
            </Section>
          )}

          {reviewChanges && keptSections}

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
          </>)}
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
              {accept}
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  )
}
