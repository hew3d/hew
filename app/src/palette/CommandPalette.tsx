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
import { Trans, useLingui } from '@lingui/react/macro'
import { TOOL_ICON_SVG } from '../tools/toolIcons'
import { InlineIcon } from '../panels/InlineIcon'
import boltSvg from '@material-symbols/svg-400/outlined/bolt.svg?raw'
import { paletteEntries, paletteShortcut, type PaletteEntry, type PaletteGate, type PaletteGroup } from './registry'
import { rankEntries } from './search'
import { getRecent, recordRun, subscribe as subscribeRecent } from './recency'
import { useKeymap } from '../keymap/useKeymap'
import { isComposingKey } from '../input/keyboard'

export interface CommandPaletteProps {
  open: boolean
  onClose: () => void
  /** Run the given menu-action id (App.tsx wires this to `menuActionRef.current`). */
  onRun: (id: string) => void
  /** Dynamic per-document entries (Model group: object/group/component/tag
   * names built by App.tsx) searched alongside the static registry. */
  extraEntries?: PaletteEntry[]
  /** Current values of the selection gates (App.tsx's menuGates + a plain
   * has-selection flag). A gated entry whose flag is false (or missing)
   * renders disabled — listed but not runnable, like a greyed menu item. */
  gates?: Partial<Record<PaletteGate, boolean>>
  /** Query pre-filled when the palette opens (Help ▸ Search hands its typed
   * text over). Empty/omitted = the usual clean slate. */
  initialQuery?: string
}

// oxlint-disable-next-line lingui/no-unlocalized-strings -- group IDs; the header text is resolved by groupTitle()
const GROUP_ORDER: PaletteGroup[] = ['Tools', 'Actions', 'Model', 'Library']

function RowIcon({ entry }: { entry: PaletteEntry }) {
  const raw = entry.toolName === undefined ? boltSvg : TOOL_ICON_SVG[entry.toolName]
  return <InlineIcon svg={raw ?? boltSvg} size={17} />
}

export function CommandPalette({ open, onClose, onRun, extraEntries, gates, initialQuery }: CommandPaletteProps) {
  const { t, i18n } = useLingui()
  // Re-render on a keymap change so the shortcuts shown stay true.
  useKeymap()
  const [query, setQuery] = useState('')
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [recentIds, setRecentIds] = useState<string[]>(() => getRecent())
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  // Where the pointer last really moved to. Chromium re-fires mouseenter on
  // whichever row ends up under a STILL pointer whenever the list re-ranks or
  // scrolls, which would steal the keyboard selection — so hover only
  // selects once the pointer has actually changed position.
  const pointerRef = useRef<{ x: number; y: number } | null>(null)

  useEffect(() => subscribeRecent(setRecentIds), [])

  // Reset every time the palette opens — to the seed if one was handed
  // over (Help ▸ Search), else a clean slate — and focus the input.
  // `initialQuery` deliberately isn't a dependency: it only matters at the
  // moment of opening.
  useEffect(() => {
    if (!open) return
    setQuery(initialQuery ?? '')
    setSelectedIndex(0)
    pointerRef.current = null
    const id = requestAnimationFrame(() => inputRef.current?.focus())
    return () => cancelAnimationFrame(id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  const allEntries = useMemo(
    () => [...paletteEntries(), ...(extraEntries ?? [])],
    // `i18n.locale`: the entries carry resolved text, so rebuild on a language switch.
    [extraEntries, i18n.locale],
  )
  const ranked = useMemo(() => rankEntries(query, allEntries, recentIds), [query, allEntries, recentIds])
  // The rows as the user SEES them: ranked order, regrouped under the group
  // headers. `selectedIndex` counts these rows, so arrows, Enter, and
  // scrolling must all index this list — not `ranked`, which interleaves
  // groups (a recently-run Action outranks Tools but is drawn below them).
  const shown = useMemo(
    () => GROUP_ORDER.flatMap((group) => ranked.filter((e) => e.group === group)),
    [ranked],
  )

  // Clamp selection whenever the ranked list changes shape (e.g. typing narrows it).
  useEffect(() => {
    setSelectedIndex((i) => Math.max(0, Math.min(i, shown.length - 1)))
  }, [shown.length])

  // Keep the keyboard selection visible as arrows walk past the fold.
  useEffect(() => {
    if (!open) return
    const row = listRef.current?.querySelector('[role="option"][aria-selected="true"]')
    // jsdom has no scrollIntoView.
    row?.scrollIntoView?.({ block: 'nearest' })
  }, [open, selectedIndex, shown])

  const isDisabled = (entry: PaletteEntry): boolean =>
    entry.gate !== undefined && (gates?.[entry.gate] ?? false) === false

  const run = (entry: PaletteEntry) => {
    if (isDisabled(entry)) return
    recordRun(entry.id)
    onRun(entry.id)
    onClose()
  }

  // The palette owns the keyboard while it is open, wherever focus is. The
  // handler sits on `window` in the capture phase: if focus has left the
  // search field (a click on the header or scrollbar, a WebView quirk), the
  // arrows, Enter, and Escape still work, and neither they nor a stray
  // letter reach the global tool shortcuts. Latest state is read through a
  // ref so the listener is attached once per open.
  const liveRef = useRef({ shown, selectedIndex, run, onClose })
  liveRef.current = { shown, selectedIndex, run, onClose }
  useEffect(() => {
    if (!open) return
    const onKeyDown = (ev: KeyboardEvent) => {
      if (isComposingKey(ev)) return
      const live = liveRef.current
      const swallow = () => {
        ev.preventDefault()
        ev.stopPropagation()
      }
      if (ev.key === 'Escape') {
        swallow()
        live.onClose()
        return
      }
      if (ev.key === 'ArrowDown') {
        swallow()
        setSelectedIndex((i) => Math.min(i + 1, live.shown.length - 1))
        return
      }
      if (ev.key === 'ArrowUp') {
        swallow()
        setSelectedIndex((i) => Math.max(i - 1, 0))
        return
      }
      if (ev.key === 'Enter') {
        swallow()
        const entry = live.shown[live.selectedIndex]
        if (entry !== undefined) live.run(entry)
        return
      }
      // Anything else typed while focus is elsewhere belongs in the search
      // field: put focus back (the browser then types the character there)
      // and keep the key from also switching tools. Chords stay the app's.
      const input = inputRef.current
      if (input === null || ev.target === input) return
      if (ev.ctrlKey || ev.metaKey || ev.altKey) return
      if (ev.key.length === 1 || ev.key === 'Backspace' || ev.key === 'Delete') {
        input.focus()
        ev.stopPropagation()
      }
    }
    window.addEventListener('keydown', onKeyDown, true)
    return () => window.removeEventListener('keydown', onKeyDown, true)
  }, [open])

  if (!open) return null

  // Group the ranked (already-ordered) list into sections, preserving order.
  const grouped = GROUP_ORDER.map((group) => ({
    group,
    items: shown.filter((e) => e.group === group),
  })).filter((g) => g.items.length > 0)

  const groupTitle = (group: PaletteGroup): string => {
    switch (group) {
      case 'Tools':
        return t`Tools`
      case 'Actions':
        return t`Actions`
      case 'Model':
        return t`Model`
      case 'Library':
        return t`Library`
    }
  }
  const recentLabels = recentIds
    .map((id) => allEntries.find((e) => e.id === id)?.label)
    .filter((label): label is string => label !== undefined)
    .slice(0, 3)
    .join(', ')

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
        aria-label={t`Command palette`}
        onMouseDown={(e) => {
          e.stopPropagation()
          // Keep focus in the search input: the key handler lives on this
          // dialog, so a click on dead space (header padding, scrollbar,
          // section titles) must not strand focus on the page, where arrows
          // and Enter do nothing and bare letters switch tools. Rows still
          // get their click.
          if (e.target !== inputRef.current) e.preventDefault()
        }}
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
            placeholder={t`Search tools, actions, help…`}
            aria-label={t`Search`}
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
            <Trans>esc to close</Trans>
          </span>
        </div>

        {/* Results */}
        <div ref={listRef} style={{ overflowY: 'auto', padding: 'var(--space-3) 0' }}>
          {grouped.length === 0 && (
            <div style={{ padding: 'var(--space-7)', textAlign: 'center', color: 'var(--text-faint)', fontSize: '13px' }}>
              <Trans>No matches for “{query}”.</Trans>
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
                {groupTitle(group)}
              </div>
              {items.map((entry) => {
                flatIndex += 1
                const index = flatIndex
                const selected = index === selectedIndex
                const disabled = isDisabled(entry)
                const shortcut = paletteShortcut(entry)
                return (
                  <div
                    key={entry.id}
                    role="option"
                    aria-selected={selected}
                    aria-disabled={disabled || undefined}
                    onMouseMove={(ev) => {
                      const last = pointerRef.current
                      if (last !== null && last.x === ev.clientX && last.y === ev.clientY) return
                      pointerRef.current = { x: ev.clientX, y: ev.clientY }
                      setSelectedIndex(index)
                    }}
                    onClick={() => run(entry)}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 'var(--space-4)',
                      padding: '7px var(--space-6)',
                      cursor: disabled ? 'default' : 'pointer',
                      opacity: disabled ? 0.45 : 1,
                      background: selected ? 'var(--accent-tint-18)' : 'transparent',
                    }}
                  >
                    <RowIcon entry={entry} />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div
                        style={{
                          fontSize: 'var(--font-size-palette-title)',
                          fontWeight: 600,
                          color: selected && !disabled ? 'var(--accent-text-strong)' : 'var(--text-primary)',
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
                    {disabled && (
                      <span
                        style={{
                          fontSize: 'var(--font-size-palette-desc)',
                          color: 'var(--text-faint)',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        <Trans>needs a selection</Trans>
                      </span>
                    )}
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
          <span><Trans>↑ ↓ navigate · ↵ run · esc close</Trans></span>
          {recentIds.length > 0 && query === '' && (
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: '50%' }}>
              {t`Recent: ${recentLabels}`}
            </span>
          )}
        </div>
      </div>
    </div>
  )
}
