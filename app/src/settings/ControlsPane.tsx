/**
 * ControlsPane — the "Controls" settings pane: how Hew answers the keyboard
 * (and, later, the mouse).
 *
 * Keyboard shortcuts: every command the command palette knows, in the
 * palette's own look (search on top, Tools then Actions, icon + name, the
 * keys on the right), each with its chords from the keymap (keymap/).
 * "+" records a new chord: the next keystroke with a real key is taken, Esc
 * cancels. A chord another command already uses asks before moving it
 * across; Esc, Enter, Tab and the arrows are refused (the tools' own
 * gesture keys), as are the macOS system chords (Cmd+Q, H, M) and, on the
 * macOS desktop, the fixed native Edit chords (Cmd+Z, X, C, V, A and their
 * Shift variants), which also cannot be removed from their own command. A command whose keys differ from the defaults is marked and
 * gets a Reset; Reset All restores every default.
 *
 * Changes apply at once and reach every window: the keymap persists through
 * the same setting channel as the other panes.
 */

import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'
import { Trans, useLingui } from '@lingui/react/macro'
import boltSvg from '@material-symbols/svg-400/outlined/bolt.svg?raw'
import { TOOL_ICON_SVG } from '../tools/toolIcons'
import { InlineIcon } from '../panels/InlineIcon'
import { paletteEntries, type PaletteEntry } from '../palette/registry'
import { rankEntries } from '../palette/search'
import { isComposingKey } from '../input/keyboard'
import { isTauri } from '../io/fileHost'
import { isMac } from '../platform'
import { chordFromEvent, chordToString, formatChordString } from '../keymap/chord'
import {
  boundCommandIds,
  chordsFor,
  commandForChord,
  isCustomized,
  isMacSystemChord,
  isPinnedTo,
  isReservedChord,
  pinnedOwner,
  resetAllChords,
  resetChords,
  setChords,
  setRecordingChord,
} from '../keymap/keymap'
import { useKeymap } from '../keymap/useKeymap'

const sectionHeaderStyle: CSSProperties = {
  fontFamily: 'var(--font-family-mono)',
  fontSize: 'var(--font-size-section-header, 10px)',
  fontWeight: 600,
  textTransform: 'uppercase',
  letterSpacing: '0.12em',
  color: 'var(--text-section, #999)',
  padding: '10px 0 4px',
}

const kbdStyle: CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: '3px',
  fontFamily: 'var(--font-family-mono)',
  fontSize: 'var(--font-size-kbd, 11px)',
  fontWeight: 600,
  color: 'var(--kbd-text)',
  background: 'var(--kbd-bg)',
  border: '1px solid var(--kbd-border)',
  borderRadius: 'var(--radius-kbd, 4px)',
  padding: '1.5px 5px',
  whiteSpace: 'nowrap',
}

const iconButtonStyle: CSSProperties = {
  border: 'none',
  background: 'transparent',
  color: 'var(--text-faint, #888)',
  cursor: 'pointer',
  padding: '0 2px',
  fontSize: '12px',
  lineHeight: 1,
  fontFamily: 'inherit',
}

const textButtonStyle: CSSProperties = {
  border: '1px solid var(--border-strong, #444)',
  background: 'var(--surface-input, #2a2a2a)',
  color: 'var(--text-primary, #eee)',
  borderRadius: '6px',
  padding: '3px 10px',
  fontSize: '12px',
  fontFamily: 'inherit',
  cursor: 'pointer',
  whiteSpace: 'nowrap',
}

const linkButtonStyle: CSSProperties = {
  ...iconButtonStyle,
  color: 'var(--accent-text, #8fb0ff)',
  fontSize: '11px',
}

/** Commands bindable from the keyboard but not listed by the palette. */
// oxlint-disable lingui/no-unlocalized-strings -- command ids
const EXTRA_COMMANDS: { id: string; label: MessageDescriptor; description: MessageDescriptor; desktopOnly?: boolean }[] = [
  { id: 'open-palette', label: msg`Command Palette…`, description: msg`Search every tool and action.` },
  { id: 'close', label: msg`Close Window`, description: msg`Close this document window.`, desktopOnly: true },
]
// oxlint-enable lingui/no-unlocalized-strings

function extraEntries(resolve: (d: MessageDescriptor) => string): PaletteEntry[] {
  return EXTRA_COMMANDS.filter((c) => isTauri || c.desktopOnly !== true).map((c) => ({
    id: c.id,
    label: resolve(c.label),
    description: resolve(c.description),
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- group id
    group: 'Actions',
  }))
}

function RowIcon({ entry }: { entry: PaletteEntry }) {
  const raw = entry.toolName === undefined ? boltSvg : TOOL_ICON_SVG[entry.toolName]
  return <InlineIcon svg={raw ?? boltSvg} size={17} />
}

type Capture =
  | { id: string; phase: 'listening' }
  | { id: string; phase: 'conflict'; chord: string; owner: string }
  | { id: string; phase: 'refused'; chord: string; reason: 'tools' | 'system' | 'pinned'; owner?: string }

export function ControlsPane() {
  const { t, i18n } = useLingui()
  useKeymap()
  const [query, setQuery] = useState('')
  const [capture, setCapture] = useState<Capture | null>(null)

  const entries = useMemo(
    () => [...paletteEntries(), ...extraEntries((d) => i18n._(d))],
    // `i18n.locale`: the entries carry resolved text.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [i18n.locale],
  )
  const labelOf = (id: string): string => entries.find((e) => e.id === id)?.label ?? id
  const shown = useMemo(
    () => (query.trim() === '' ? entries : rankEntries(query, entries, [])),
    [query, entries],
  )
  const anyCustomized = boundCommandIds().some(isCustomized)

  // While listening, the next real keystroke is the new chord. Capture phase
  // on window, and stopped there, so no shortcut (the app's dispatcher in the
  // web build's modal) and no tool ever sees it. While a conflict or refusal
  // is being answered the keys stay out of the app as well (Space on the
  // focused Reassign button must not reach the Select tool, nor Delete the
  // selection behind the pane); they are left their default action, so
  // Tab moves between the prompt's buttons and Space or Enter presses one.
  const captureRef = useRef(capture)
  captureRef.current = capture
  const phase = capture?.phase
  useEffect(() => {
    if (phase === undefined) return
    const onKeyDown = (ev: KeyboardEvent) => {
      if (isComposingKey(ev)) return
      const cur = captureRef.current
      if (cur === null) return
      // The search box keeps its typing in every phase.
      const target = ev.target as HTMLElement | null
      if (target !== null && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable === true)) return
      const plainEscape =
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- KeyboardEvent.key identifier
        ev.key === 'Escape' && !ev.ctrlKey && !ev.altKey && !ev.shiftKey && !ev.metaKey
      if (cur.phase !== 'listening') {
        ev.stopImmediatePropagation()
        const el = ev.target as HTMLElement | null
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- KeyboardEvent.key identifier
        if (ev.key === 'Backspace' && el?.tagName !== 'INPUT' && el?.tagName !== 'TEXTAREA') ev.preventDefault()
        if (plainEscape) {
          ev.preventDefault()
          setCapture(null)
        }
        return
      }
      ev.preventDefault()
      ev.stopImmediatePropagation()
      if (plainEscape) {
        setCapture(null)
        return
      }
      const chord = chordFromEvent(ev, isMac)
      if (chord === null) return // a modifier on its own: keep listening
      const text = chordToString(chord)
      if (isReservedChord(text)) {
        setCapture({ id: cur.id, phase: 'refused', chord: text, reason: 'tools' })
        return
      }
      if (isMacSystemChord(text)) {
        setCapture({ id: cur.id, phase: 'refused', chord: text, reason: 'system' })
        return
      }
      // A fixed native menu accelerator (macOS desktop) stays with its item.
      const pinned = pinnedOwner(text)
      if (pinned !== undefined && pinned !== cur.id) {
        setCapture({ id: cur.id, phase: 'refused', chord: text, reason: 'pinned', owner: pinned })
        return
      }
      const owner = commandForChord(text)
      if (owner === cur.id) {
        setCapture(null)
        return
      }
      if (owner !== undefined) {
        setCapture({ id: cur.id, phase: 'conflict', chord: text, owner })
        return
      }
      setChords(cur.id, [...chordsFor(cur.id), text])
      setCapture(null)
    }
    window.addEventListener('keydown', onKeyDown, true)
    setRecordingChord(true)
    return () => {
      window.removeEventListener('keydown', onKeyDown, true)
      setRecordingChord(false)
    }
  }, [phase])

  const groups = [
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- group ids
    { group: 'Tools', title: t`Tools` },
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- group ids
    { group: 'Actions', title: t`Actions` },
  ] as const

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '6px', fontSize: '13px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: '10px', justifyContent: 'space-between' }}>
        <div style={{ ...sectionHeaderStyle, padding: 0 }}>
          <Trans>Keyboard shortcuts</Trans>
        </div>
        <button
          type="button"
          onClick={() => {
            resetAllChords()
            setCapture(null)
          }}
          disabled={!anyCustomized}
          style={{ ...textButtonStyle, opacity: anyCustomized ? 1 : 0.45, cursor: anyCustomized ? 'pointer' : 'default' }}
        >
          <Trans>Reset All</Trans>
        </button>
      </div>

      <input
        type="search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={t`Search commands…`}
        aria-label={t`Search commands`}
        style={{
          padding: '5px 8px',
          fontSize: '13px',
          background: 'var(--surface-input, #2a2a2a)',
          color: 'var(--text-primary, #eee)',
          border: '1px solid var(--border-strong, #444)',
          borderRadius: '6px',
          fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
        }}
      />

      {shown.length === 0 && (
        <div style={{ padding: '16px', textAlign: 'center', color: 'var(--text-faint, #888)' }}>
          <Trans>No matches for “{query}”.</Trans>
        </div>
      )}

      <div role="list" aria-label={t`Keyboard shortcuts`}>
        {groups.map(({ group, title }) => {
          const items = shown.filter((e) => e.group === group)
          if (items.length === 0) return null
          return (
            <div key={group}>
              <div style={sectionHeaderStyle}>{title}</div>
              {items.map((entry) => (
                <ShortcutRow
                  key={entry.id}
                  entry={entry}
                  capture={capture?.id === entry.id ? capture : null}
                  ownerLabel={
                    capture?.id === entry.id && capture.phase === 'conflict'
                      ? labelOf(capture.owner)
                      : capture?.id === entry.id && capture.phase === 'refused' && capture.owner !== undefined
                        ? labelOf(capture.owner)
                        : ''
                  }
                  onListen={() => setCapture({ id: entry.id, phase: 'listening' })}
                  onCancel={() => setCapture(null)}
                />
              ))}
            </div>
          )
        })}
      </div>

      <div style={{ fontSize: '11px', lineHeight: 1.5, color: 'var(--text-faint, #888)', paddingTop: '6px' }}>
        <Trans>
          Esc, Enter, Tab and the arrow keys stay with the tools (cancel, commit, axis lock), and so does
          the Measurements box while you type a value.
        </Trans>
      </div>
    </div>
  )
}

function ShortcutRow({
  entry,
  capture,
  ownerLabel,
  onListen,
  onCancel,
}: {
  entry: PaletteEntry
  capture: Capture | null
  ownerLabel: string
  onListen: () => void
  onCancel: () => void
}) {
  const { t } = useLingui()
  const [hovered, setHovered] = useState(false)
  // Keyboard users reach the row's buttons with Tab: focus inside the row
  // shows them, as hovering does.
  const [focusWithin, setFocusWithin] = useState(false)
  const showControls = hovered || focusWithin
  const chords = chordsFor(entry.id)
  const customized = isCustomized(entry.id)
  const label = entry.label
  const chordLabel = capture !== null && capture.phase !== 'listening' ? formatChordString(capture.chord, isMac) : ''

  return (
    <div
      role="listitem"
      aria-label={label}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocusWithin(true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocusWithin(false)
      }}
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: '4px',
        padding: '6px 8px',
        borderRadius: 'var(--radius-control, 6px)',
        background: capture !== null ? 'var(--accent-tint-15, rgba(91,140,255,0.15))' : hovered ? 'var(--surface-hover, rgba(128,128,128,0.08))' : 'transparent',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
        <RowIcon entry={entry} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontWeight: 600,
              color: 'var(--text-primary, #eee)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {label}
            {customized && (
              <span
                title={t`Changed from the default`}
                aria-label={t`Changed from the default`}
                style={{ color: 'var(--accent-base, #5b8cff)', marginLeft: '6px' }}
              >
                ●
              </span>
            )}
          </div>
          <div
            style={{
              fontSize: '11px',
              color: 'var(--text-muted, #aaa)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {entry.description}
          </div>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: '4px', flexWrap: 'wrap', justifyContent: 'flex-end' }}>
          {chords.map((c) => {
            const shown = formatChordString(c, isMac)
            // The fixed native chord of this command (macOS desktop) cannot
            // be removed: the menu item would keep firing it.
            const pinned = isPinnedTo(entry.id, c)
            return (
              <span key={c} style={kbdStyle}>
                {shown}
                {!pinned && (
                  <button
                    type="button"
                    aria-label={t`Remove ${shown} from ${label}`}
                    title={t`Remove`}
                    onClick={() => setChords(entry.id, chords.filter((x) => x !== c))}
                    style={{ ...iconButtonStyle, display: showControls ? 'inline' : 'none' }}
                  >
                    ×
                  </button>
                )}
              </span>
            )
          })}
          {capture?.phase === 'listening' ? (
            <span style={{ ...kbdStyle, color: 'var(--accent-text, #8fb0ff)' }}>
              <Trans>Press a key…</Trans>
              <button type="button" aria-label={t`Cancel`} onClick={onCancel} style={iconButtonStyle}>
                ×
              </button>
            </span>
          ) : (
            <button
              type="button"
              aria-label={t`Add a shortcut to ${label}`}
              title={t`Add a shortcut`}
              onClick={onListen}
              style={{ ...kbdStyle, cursor: 'pointer', opacity: showControls ? 1 : 0 }}
            >
              +
            </button>
          )}
          {customized && (
            <button type="button" onClick={() => resetChords(entry.id)} style={linkButtonStyle}>
              <Trans>Reset</Trans>
            </button>
          )}
        </div>
      </div>

      {capture?.phase === 'conflict' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', paddingLeft: '27px', fontSize: '12px' }}>
          <span style={{ color: 'var(--text-secondary, #ddd)' }}>
            <Trans>
              {chordLabel} is used by {ownerLabel}.
            </Trans>
          </span>
          <button
            type="button"
            autoFocus
            onClick={() => {
              setChords(entry.id, [...chords, capture.chord])
              onCancel()
            }}
            style={textButtonStyle}
          >
            <Trans>Reassign</Trans>
          </button>
          <button type="button" onClick={onCancel} style={textButtonStyle}>
            <Trans>Cancel</Trans>
          </button>
        </div>
      )}
      {capture?.phase === 'refused' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', paddingLeft: '27px', fontSize: '12px' }}>
          <span style={{ color: 'var(--text-secondary, #ddd)' }}>
            {capture.reason === 'system' ? (
              <Trans>
                {chordLabel} is reserved by macOS and can’t be a shortcut.
              </Trans>
            ) : capture.reason === 'pinned' ? (
              <Trans>
                {chordLabel} stays with {ownerLabel} on this Mac and can’t be a shortcut.
              </Trans>
            ) : (
              <Trans>
                {chordLabel} belongs to the tools and can’t be a shortcut.
              </Trans>
            )}
          </span>
          <button type="button" autoFocus onClick={onListen} style={textButtonStyle}>
            <Trans>Try Another</Trans>
          </button>
          <button type="button" onClick={onCancel} style={textButtonStyle}>
            <Trans>Cancel</Trans>
          </button>
        </div>
      )}
    </div>
  )
}
