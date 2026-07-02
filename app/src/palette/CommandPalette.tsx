/**
 * CommandPalette — the ⌘K / Ctrl-K modal (`04_command_palette.md`).
 *
 * A single searchable entry point to every tool and action `paletteEntries()`
 * knows about. Selecting a result calls `onRun(entry.id)`, which the caller
 * (App.tsx) wires straight to the existing `menuActionRef.current(id)`
 * dispatch — this component owns no execution logic, only search + keyboard
 * navigation + presentation.
 *
 * Components (from the tray) and Learn (real help content) sections
 * from the spec are deferred — see `palette/registry.ts`'s doc comment.
 */

import { useEffect, useMemo, useRef, useState } from 'react'
import { TOOL_ICON_SVG } from '../tools/toolIcons'
import boltSvg from '@material-symbols/svg-400/outlined/bolt.svg?raw'
import { paletteEntries, paletteShortcut, type PaletteEntry, type PaletteGroup } from './registry'
import { rankEntries } from './search'
import { getRecent, recordRun, subscribe as subscribeRecent } from './recency'
import { isMac } from '../platform'

export interface CommandPaletteProps {
  open: boolean
  onClose: () => void
  /** Run the given menu-action id (App.tsx wires this to `menuActionRef.current`). */
  onRun: (id: string) => void
}

const GROUP_ORDER: PaletteGroup[] = ['Tools', 'Actions']

function RowIcon({ entry }: { entry: PaletteEntry }) {
  const raw = entry.group === 'Tools' ? TOOL_ICON_SVG[entry.label as keyof typeof TOOL_ICON_SVG] : boltSvg
  const svg = (raw ?? boltSvg)
    .replace(/\swidth="[^"]*"/, '')
    .replace(/\sheight="[^"]*"/, '')
    .replace('<svg ', '<svg fill="currentColor" width="17" height="17" ')
  return (
    <span
      aria-hidden="true"
      style={{ width: '17px', height: '17px', display: 'block', overflow: 'hidden', flexShrink: 0 }}
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}

export function CommandPalette({ open, onClose, onRun }: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [recentIds, setRecentIds] = useState<string[]>(() => getRecent())
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => subscribeRecent(setRecentIds), [])

  // Reset to a clean slate every time the palette opens, and focus the input.
  useEffect(() => {
    if (!open) return
    setQuery('')
    setSelectedIndex(0)
    const id = requestAnimationFrame(() => inputRef.current?.focus())
    return () => cancelAnimationFrame(id)
  }, [open])

  const allEntries = useMemo(() => paletteEntries(), [])
  const ranked = useMemo(() => rankEntries(query, allEntries, recentIds), [query, allEntries, recentIds])

  // Clamp selection whenever the ranked list changes shape (e.g. typing narrows it).
  useEffect(() => {
    setSelectedIndex((i) => Math.max(0, Math.min(i, ranked.length - 1)))
  }, [ranked.length])

  const run = (entry: PaletteEntry) => {
    recordRun(entry.id)
    onRun(entry.id)
    onClose()
  }

  const onKeyDown = (ev: React.KeyboardEvent) => {
    if (ev.key === 'Escape') {
      ev.preventDefault()
      onClose()
      return
    }
    if (ev.key === 'ArrowDown') {
      ev.preventDefault()
      setSelectedIndex((i) => Math.min(i + 1, ranked.length - 1))
      return
    }
    if (ev.key === 'ArrowUp') {
      ev.preventDefault()
      setSelectedIndex((i) => Math.max(i - 1, 0))
      return
    }
    if (ev.key === 'Enter') {
      ev.preventDefault()
      const entry = ranked[selectedIndex]
      if (entry !== undefined) run(entry)
    }
  }

  if (!open) return null

  // Group the ranked (already-ordered) list into sections, preserving order.
  const grouped = GROUP_ORDER.map((group) => ({
    group,
    items: ranked.filter((e) => e.group === group),
  })).filter((g) => g.items.length > 0)

  let flatIndex = -1

  return (
    <div
      onMouseDown={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'var(--backdrop-dim)',
        backdropFilter: 'blur(8px)',
        zIndex: 500,
        display: 'flex',
        justifyContent: 'center',
        paddingTop: '70px',
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
        style={{
          width: '550px',
          maxHeight: '70vh',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--surface-window)',
          border: '1px solid var(--border-strong)',
          borderRadius: 'var(--radius-palette)',
          boxShadow: 'var(--shadow-palette)',
          overflow: 'hidden',
        }}
      >
        {/* Search header */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 'var(--space-4)',
            padding: 'var(--space-6)',
            borderBottom: '1px solid var(--border-hairline)',
          }}
        >
          <span aria-hidden="true" style={{ color: 'var(--text-faint)', fontSize: '15px' }}>⌕</span>
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search tools, actions, help…"
            aria-label="Search"
            style={{
              flex: 1,
              border: 'none',
              outline: 'none',
              background: 'transparent',
              fontFamily: 'var(--font-family-ui)',
              fontSize: '16px',
              color: 'var(--text-primary)',
              caretColor: 'var(--accent-base)',
            }}
          />
          <span
            style={{
              fontSize: 'var(--font-size-body)',
              color: 'var(--text-faint)',
              whiteSpace: 'nowrap',
            }}
          >
            esc to close
          </span>
        </div>

        {/* Results */}
        <div style={{ overflowY: 'auto', padding: 'var(--space-3) 0' }}>
          {grouped.length === 0 && (
            <div style={{ padding: 'var(--space-7)', textAlign: 'center', color: 'var(--text-faint)', fontSize: '13px' }}>
              No matches for “{query}”.
            </div>
          )}
          {grouped.map(({ group, items }) => (
            <div key={group}>
              <div
                style={{
                  fontFamily: 'var(--font-family-mono)',
                  fontSize: 'var(--font-size-section-header)',
                  fontWeight: 600,
                  textTransform: 'uppercase',
                  letterSpacing: '0.12em',
                  color: 'var(--text-section)',
                  padding: '8px var(--space-6) 4px',
                }}
              >
                {group}
              </div>
              {items.map((entry) => {
                flatIndex += 1
                const selected = flatIndex === selectedIndex
                const shortcut = paletteShortcut(entry, isMac)
                return (
                  <div
                    key={entry.id}
                    role="option"
                    aria-selected={selected}
                    onMouseEnter={() => setSelectedIndex(flatIndex)}
                    onClick={() => run(entry)}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 'var(--space-4)',
                      padding: '7px var(--space-6)',
                      cursor: 'pointer',
                      background: selected ? 'var(--accent-tint-18)' : 'transparent',
                    }}
                  >
                    <RowIcon entry={entry} />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div
                        style={{
                          fontSize: 'var(--font-size-palette-title)',
                          fontWeight: 600,
                          color: selected ? 'var(--accent-text-strong)' : 'var(--text-primary)',
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {entry.label}
                      </div>
                      <div
                        style={{
                          fontSize: 'var(--font-size-palette-desc)',
                          color: 'var(--text-muted)',
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {entry.description}
                      </div>
                    </div>
                    {shortcut !== '' && (
                      <span
                        style={{
                          fontFamily: 'var(--font-family-mono)',
                          fontSize: 'var(--font-size-kbd)',
                          fontWeight: 600,
                          color: selected ? 'var(--kbd-active-text)' : 'var(--kbd-text)',
                          background: selected ? 'var(--kbd-active-bg)' : 'var(--kbd-bg)',
                          border: `1px solid ${selected ? 'transparent' : 'var(--kbd-border)'}`,
                          borderRadius: 'var(--radius-kbd)',
                          padding: '1.5px 5px',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {shortcut}
                      </span>
                    )}
                  </div>
                )
              })}
            </div>
          ))}
        </div>

        {/* Footer */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: '8px var(--space-6)',
            borderTop: '1px solid var(--border-hairline)',
            fontSize: 'var(--font-size-body)',
            color: 'var(--text-faint)',
          }}
        >
          <span>↑ ↓ navigate · ↵ run · esc close</span>
          {recentIds.length > 0 && query === '' && (
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: '50%' }}>
              Recent: {recentIds
                .map((id) => allEntries.find((e) => e.id === id)?.label)
                .filter((label): label is string => label !== undefined)
                .slice(0, 3)
                .join(', ')}
            </span>
          )}
        </div>
      </div>
    </div>
  )
}
